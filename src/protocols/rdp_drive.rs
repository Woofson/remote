//! Guarded RDP drive redirection.
//!
//! The upstream native drive backend joins whatever path the remote Windows machine
//! sends onto its base folder without any checks, so a compromised or hostile remote
//! machine could reach files outside it (`\..\..\secret.key`). This wrapper sits in
//! front of it: one inner backend per redirected drive, every path is checked to stay
//! inside that drive's folder (also through symlinks), and read-only drives refuse
//! anything that writes, renames or deletes.

use ironrdp_core::impl_as_any;
use ironrdp_pdu::{encode_err, PduResult};
use ironrdp_rdpdr::pdu::efs::*;
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr_native::backend::NixRdpdrBackend;
use ironrdp_svc::SvcMessage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

/// One drive offered to the remote machine.
#[derive(Debug, Clone)]
pub struct DriveSpec {
    pub device_id: u32,
    /// Name shown in Windows (`\\tsclient\<name>`).
    pub name: String,
    /// Folder on the Remote server.
    pub path: PathBuf,
    pub read_only: bool,
}

#[derive(Debug)]
struct Drive {
    inner: NixRdpdrBackend,
    base: PathBuf,
    read_only: bool,
}

#[derive(Debug)]
pub struct GuardedDriveBackend {
    drives: HashMap<u32, Drive>,
}

impl_as_any!(GuardedDriveBackend);

impl GuardedDriveBackend {
    pub fn new(specs: &[DriveSpec]) -> Self {
        let drives = specs
            .iter()
            .filter_map(|s| {
                let _ = std::fs::create_dir_all(&s.path);
                let base = std::fs::canonicalize(&s.path).ok()?;
                Some((
                    s.device_id,
                    Drive {
                        inner: NixRdpdrBackend::new(base.to_string_lossy().into_owned()),
                        base,
                        read_only: s.read_only,
                    },
                ))
            })
            .collect();
        Self { drives }
    }
}

/// Is a path sent by Windows (e.g. `\dir\file.txt`) safely inside `base`?
pub fn path_is_inside(base: &Path, windows_path: &str) -> bool {
    let rel = windows_path.replace('\\', "/");
    for part in rel.split('/') {
        if part == ".." || part.contains('\0') || part.contains(':') {
            return false;
        }
    }
    // Directory queries end in a wildcard; check the folder part.
    let rel = rel.trim_end_matches('*');
    let full = base.join(rel.trim_start_matches('/'));

    // The deepest existing part must resolve (through symlinks) to inside base.
    let mut probe: &Path = &full;
    loop {
        if std::fs::symlink_metadata(probe).is_ok() {
            return std::fs::canonicalize(probe).map(|c| c.starts_with(base)).unwrap_or(false);
        }
        match probe.parent() {
            Some(p) if p.starts_with(base) => probe = p,
            _ => return false,
        }
    }
}

fn device_io(req: &ServerDriveIoRequest) -> &DeviceIoRequest {
    match req {
        ServerDriveIoRequest::ServerCreateDriveRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceCloseRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceControlRequest(r) => &r.header,
        ServerDriveIoRequest::DeviceReadRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceWriteRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveSetInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveLockControlRequest(r) => &r.device_io_request,
    }
}

const WRITE_ACCESS: DesiredAccess = DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE
    .union(DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY)
    .union(DesiredAccess::FILE_WRITE_EA)
    .union(DesiredAccess::FILE_DELETE_CHILD)
    .union(DesiredAccess::FILE_WRITE_ATTRIBUTES)
    .union(DesiredAccess::DELETE)
    .union(DesiredAccess::WRITE_DAC)
    .union(DesiredAccess::WRITE_OWNER)
    .union(DesiredAccess::GENERIC_WRITE)
    .union(DesiredAccess::GENERIC_ALL);

/// Would this create/open request change anything on disk?
fn create_writes(r: &DeviceCreateRequest) -> bool {
    r.create_disposition != CreateDisposition::FILE_OPEN
        || r.desired_access.intersects(WRITE_ACCESS)
        || r.create_options.contains(CreateOptions::FILE_DELETE_ON_CLOSE)
}

fn deny(req: ServerDriveIoRequest, status: NtStatus) -> PduResult<Vec<SvcMessage>> {
    let pdu = match req {
        ServerDriveIoRequest::ServerCreateDriveRequest(r) => RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
            device_io_reply: DeviceIoResponse::new(r.device_io_request, status),
            file_id: 0,
            information: Information::empty(),
        }),
        ServerDriveIoRequest::DeviceWriteRequest(r) => RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
            device_io_reply: DeviceIoResponse::new(r.device_io_request, status),
            length: 0,
        }),
        ServerDriveIoRequest::ServerDriveSetInformationRequest(r) => RdpdrPdu::ClientDriveSetInformationResponse(
            ClientDriveSetInformationResponse::new(&r, status).map_err(|e| encode_err!(e))?,
        ),
        ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(r) => {
            RdpdrPdu::ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse {
                device_io_reply: DeviceIoResponse::new(r.device_io_request, status),
                buffer: None,
            })
        }
        // Only the request kinds above are ever refused.
        _ => return Ok(Vec::new()),
    };
    Ok(vec![SvcMessage::from(pdu)])
}

impl RdpdrBackend for GuardedDriveBackend {
    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }

    fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
        Ok(())
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        let device_id = device_io(&req).device_id;
        let Some(drive) = self.drives.get_mut(&device_id) else {
            warn!("RDP drive: request for unknown device {}", device_id);
            return deny(req, NtStatus::NO_SUCH_FILE);
        };

        let refusal = match &req {
            ServerDriveIoRequest::ServerCreateDriveRequest(r) => {
                if !path_is_inside(&drive.base, &r.path) {
                    Some(("path outside the drive", NtStatus::ACCESS_DENIED))
                } else if drive.read_only && create_writes(r) {
                    Some(("write on a read-only drive", NtStatus::ACCESS_DENIED))
                } else {
                    None
                }
            }
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(r) => {
                (!path_is_inside(&drive.base, &r.path)).then_some(("path outside the drive", NtStatus::NO_SUCH_FILE))
            }
            ServerDriveIoRequest::DeviceWriteRequest(_) if drive.read_only => {
                Some(("write on a read-only drive", NtStatus::ACCESS_DENIED))
            }
            ServerDriveIoRequest::ServerDriveSetInformationRequest(r) => {
                if drive.read_only {
                    Some(("change on a read-only drive", NtStatus::ACCESS_DENIED))
                } else if let FileInformationClass::Rename(info) = &r.set_buffer {
                    (!path_is_inside(&drive.base, &info.file_name))
                        .then_some(("rename outside the drive", NtStatus::ACCESS_DENIED))
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some((why, status)) = refusal {
            warn!("RDP drive: refused {} (device {})", why, device_id);
            return deny(req, status);
        }
        drive.inner.handle_drive_io_request(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_paths_inside() {
        let dir = std::env::temp_dir().join(format!("remote-drive-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let base = std::fs::canonicalize(&dir).unwrap();

        assert!(path_is_inside(&base, "\\"));
        assert!(path_is_inside(&base, "\\sub\\new-file.txt"));
        assert!(path_is_inside(&base, "\\sub\\*"));
        assert!(path_is_inside(&base, "\\does\\not\\exist\\yet"));
        assert!(!path_is_inside(&base, "\\..\\secret.key"));
        assert!(!path_is_inside(&base, "\\sub\\..\\..\\etc\\passwd"));
        assert!(!path_is_inside(&base, "\\file.txt:hidden-stream"));

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", base.join("escape")).unwrap();
            assert!(!path_is_inside(&base, "\\escape\\passwd"));
            assert!(!path_is_inside(&base, "\\escape"));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
