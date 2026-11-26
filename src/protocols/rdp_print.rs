//! RDP printing (#36): a virtual printer "Remote" in the Windows session. Windows renders
//! each job with its built-in "Microsoft Print to PDF" driver and sends the finished file
//! over the device channel; it is saved in the user's Files, in the "Prints" folder.

use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::efs::*;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_svc::SvcMessage;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

/// Device id of the printer (drives use 1 and 2).
pub const PRINTER_DEVICE_ID: u32 = 10;
/// Name of the printer in Windows.
pub const PRINTER_NAME: &str = "Remote";
/// Folder inside the user's Files where print jobs are saved.
pub const PRINTS_FOLDER: &str = "Prints";
/// A larger job is stopped, so one runaway job cannot fill the disk.
pub const MAX_JOB_BYTES: u64 = 200 * 1024 * 1024;

/// A finished (or failed) print job.
#[derive(Debug, Clone)]
pub struct PrintedJob {
    /// File name in the Prints folder (empty when the job failed).
    pub file_name: String,
    /// Path inside the user's Files, e.g. "Prints/Print 2026-11-26 14.05.03.pdf".
    pub rel_path: String,
    pub size: u64,
    /// Why the job was not saved.
    pub error: Option<String>,
}

/// Where print jobs go and who is told about them.
#[derive(Clone)]
pub struct PrinterSpec {
    /// The user's Files folder (the Prints folder is made inside it).
    pub files_root: PathBuf,
    /// Called for every finished or failed job (browser notice, activity log).
    pub on_job: Arc<dyn Fn(PrintedJob) + Send + Sync>,
}

impl std::fmt::Debug for PrinterSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrinterSpec").field("files_root", &self.files_root).finish()
    }
}

struct Job {
    part: PathBuf,
    file: Option<File>,
    size: u64,
    too_big: bool,
    failed: bool,
}

/// Receives print jobs: Create opens a part file, Write appends, Close finishes it.
pub struct PrintSink {
    spec: PrinterSpec,
    dir: PathBuf,
    jobs: HashMap<u32, Job>,
    next_id: u32,
}

impl std::fmt::Debug for PrintSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrintSink").field("dir", &self.dir).field("jobs", &self.jobs.len()).finish()
    }
}

impl PrintSink {
    pub fn new(spec: PrinterSpec) -> Self {
        let dir = spec.files_root.join(PRINTS_FOLDER);
        Self { spec, dir, jobs: HashMap::new(), next_id: 1 }
    }

    pub fn handle(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        let pdu = match req {
            PrinterIoRequest::Create(r) => {
                let (status, file_id) = match self.open_job() {
                    Ok(id) => (NtStatus::SUCCESS, id),
                    Err(e) => {
                        warn!("RDP printer: could not start a print job: {}", e);
                        (NtStatus::UNSUCCESSFUL, 0)
                    }
                };
                RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                    device_io_reply: DeviceIoResponse::new(r.device_io_request, status),
                    file_id,
                    information: if status == NtStatus::SUCCESS { Information::FILE_OPENED } else { Information::empty() },
                })
            }
            PrinterIoRequest::Write(r) => {
                let len = r.write_data.len();
                let ok = self.write_job(r.device_io_request.file_id, &r.write_data);
                RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                    device_io_reply: DeviceIoResponse::new(r.device_io_request, if ok { NtStatus::SUCCESS } else { NtStatus::UNSUCCESSFUL }),
                    length: if ok { len as u32 } else { 0 },
                })
            }
            PrinterIoRequest::Close(r) => {
                self.finish_job(r.device_io_request.file_id);
                RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
                    device_io_response: DeviceIoResponse::new(r.device_io_request, NtStatus::SUCCESS),
                })
            }
        };
        Ok(vec![SvcMessage::from(pdu)])
    }

    fn open_job(&mut self) -> std::io::Result<u32> {
        std::fs::create_dir_all(&self.dir)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let part = self.dir.join(format!(".printing-{}-{}.part", std::process::id(), uuid::Uuid::new_v4().simple()));
        let file = File::create(&part)?;
        self.jobs.insert(id, Job { part, file: Some(file), size: 0, too_big: false, failed: false });
        Ok(id)
    }

    /// Append to a job; false stops it (unknown job, too big, or a disk error).
    fn write_job(&mut self, id: u32, data: &[u8]) -> bool {
        let Some(job) = self.jobs.get_mut(&id) else {
            return false;
        };
        if job.too_big || job.failed {
            return false;
        }
        if job.size + data.len() as u64 > MAX_JOB_BYTES {
            job.too_big = true;
            job.file = None;
            return false;
        }
        match job.file.as_mut().map(|f| f.write_all(data)) {
            Some(Ok(())) => {
                job.size += data.len() as u64;
                true
            }
            _ => {
                job.failed = true;
                job.file = None;
                false
            }
        }
    }

    fn finish_job(&mut self, id: u32) {
        let Some(mut job) = self.jobs.remove(&id) else {
            return;
        };
        let flushed = job.file.take().map(|mut f| f.flush().and_then(|_| f.sync_all())).transpose();
        let failure = if job.too_big {
            Some(format!("The print job was larger than {} MB and was stopped", MAX_JOB_BYTES / (1024 * 1024)))
        } else if job.failed || flushed.is_err() {
            Some("The print job could not be saved on the Remote server".to_string())
        } else if job.size == 0 {
            // Windows opens and closes the printer without data now and then: nothing to report.
            let _ = std::fs::remove_file(&job.part);
            return;
        } else {
            None
        };
        if let Some(error) = failure {
            let _ = std::fs::remove_file(&job.part);
            warn!("RDP printer: {}", error);
            (self.spec.on_job)(PrintedJob { file_name: String::new(), rel_path: String::new(), size: job.size, error: Some(error) });
            return;
        }
        let ext = sniff_extension(&job.part);
        let name = unique_name(&self.dir, &format!("Print {}", chrono::Local::now().format("%Y-%m-%d %H.%M.%S")), ext);
        let target = self.dir.join(&name);
        if let Err(e) = std::fs::rename(&job.part, &target) {
            let _ = std::fs::remove_file(&job.part);
            warn!("RDP printer: could not save {}: {}", name, e);
            (self.spec.on_job)(PrintedJob {
                file_name: String::new(),
                rel_path: String::new(),
                size: job.size,
                error: Some("The print job could not be saved on the Remote server".into()),
            });
            return;
        }
        info!("RDP printer: saved {} ({} bytes)", name, job.size);
        (self.spec.on_job)(PrintedJob { rel_path: format!("{}/{}", PRINTS_FOLDER, name), file_name: name, size: job.size, error: None });
    }
}

impl Drop for PrintSink {
    /// Jobs still open when the session ends were never finished: remove their part files.
    fn drop(&mut self) {
        for job in self.jobs.values() {
            let _ = std::fs::remove_file(&job.part);
        }
    }
}

/// "pdf" for the Print to PDF driver; other drivers' output keeps a fitting extension.
fn sniff_extension(path: &Path) -> &'static str {
    let mut head = [0u8; 8];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"%PDF") {
        "pdf"
    } else if head.starts_with(b"%!PS") {
        "ps"
    } else if head.starts_with(b"PK") {
        "xps"
    } else {
        "prn"
    }
}

/// "Print 2026-11-26 14.05.03.pdf", or with " (2)" when that name is taken.
fn unique_name(dir: &Path, base: &str, ext: &str) -> String {
    let first = format!("{}.{}", base, ext);
    if !dir.join(&first).exists() {
        return first;
    }
    (2..1000)
        .map(|n| format!("{} ({}).{}", base, n, ext))
        .find(|n| !dir.join(n).exists())
        .unwrap_or_else(|| format!("{} {}.{}", base, uuid::Uuid::new_v4().simple(), ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_core::ReadCursor;
    use std::sync::Mutex;

    fn sink() -> (PrintSink, tempfile::TempDir, Arc<Mutex<Vec<PrintedJob>>>) {
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let spec = PrinterSpec { files_root: dir.path().to_path_buf(), on_job: Arc::new(move |j| s2.lock().unwrap().push(j)) };
        (PrintSink::new(spec), dir, seen)
    }

    fn io(major: MajorFunction, file_id: u32) -> DeviceIoRequest {
        DeviceIoRequest { device_id: PRINTER_DEVICE_ID, file_id, completion_id: 7, major_function: major, minor_function: MinorFunction::from(0u32) }
    }

    fn write(file_id: u32, data: &[u8]) -> PrinterIoRequest {
        // Length, Offset, 20 bytes padding, then the data.
        let mut raw = Vec::new();
        raw.extend_from_slice(&(data.len() as u32).to_le_bytes());
        raw.extend_from_slice(&0u64.to_le_bytes());
        raw.extend_from_slice(&[0u8; 20]);
        raw.extend_from_slice(data);
        PrinterIoRequest::Write(DeviceWriteRequest::decode(io(MajorFunction::Write, file_id), &mut ReadCursor::new(&raw)).unwrap())
    }

    fn close(file_id: u32) -> PrinterIoRequest {
        PrinterIoRequest::Close(DeviceCloseRequest::decode(io(MajorFunction::Close, file_id)))
    }

    #[test]
    fn a_job_is_saved_as_pdf() {
        let (mut s, dir, seen) = sink();
        let id = s.open_job().unwrap();
        let pdf = b"%PDF-1.7\n1 0 obj<<>>endobj\n%%EOF\n";
        s.handle(write(id, &pdf[..10])).unwrap();
        s.handle(write(id, &pdf[10..])).unwrap();
        s.handle(close(id)).unwrap();
        let jobs = seen.lock().unwrap();
        assert_eq!(jobs.len(), 1);
        let j = &jobs[0];
        assert!(j.error.is_none() && j.file_name.ends_with(".pdf") && j.rel_path.starts_with("Prints/"), "{:?}", j);
        assert_eq!(std::fs::read(dir.path().join(&j.rel_path)).unwrap(), pdf);
        let left: Vec<_> = std::fs::read_dir(dir.path().join(PRINTS_FOLDER)).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".part")).collect();
        assert!(left.is_empty(), "no part files left");
    }

    #[test]
    fn same_second_gets_a_new_name() {
        let (mut s, _dir, seen) = sink();
        for _ in 0..2 {
            let id = s.open_job().unwrap();
            s.handle(write(id, b"%PDF-1.4 x")).unwrap();
            s.handle(close(id)).unwrap();
        }
        let jobs = seen.lock().unwrap();
        assert_ne!(jobs[0].file_name, jobs[1].file_name);
    }

    #[test]
    fn empty_and_unfinished_jobs_leave_nothing() {
        let (mut s, dir, seen) = sink();
        let id = s.open_job().unwrap();
        s.handle(close(id)).unwrap();
        assert!(seen.lock().unwrap().is_empty(), "an empty job is not reported");
        let id = s.open_job().unwrap();
        s.handle(write(id, b"%PDF-1.4 half")).unwrap();
        drop(s);
        let n = std::fs::read_dir(dir.path().join(PRINTS_FOLDER)).unwrap().count();
        assert_eq!(n, 0, "the unfinished job's part file is removed when the session ends");
    }

    #[test]
    fn a_job_over_the_limit_is_stopped() {
        let (mut s, dir, seen) = sink();
        let id = s.open_job().unwrap();
        s.jobs.get_mut(&id).unwrap().size = MAX_JOB_BYTES - 4;
        assert!(s.write_job(id, b"%PD"));
        assert!(!s.write_job(id, b"more than allowed"));
        s.handle(close(id)).unwrap();
        let jobs = seen.lock().unwrap();
        assert!(jobs[0].error.as_deref().unwrap_or("").contains("larger than 200 MB"));
        assert_eq!(std::fs::read_dir(dir.path().join(PRINTS_FOLDER)).unwrap().count(), 0);
    }

    /// The whole path: raw device-channel packets, laid out as Windows sends them, through the
    /// RDP library's own handling into the drive/printer backend, down to the saved PDF.
    #[test]
    fn packets_from_windows_end_up_as_a_pdf() {
        use ironrdp_svc::SvcProcessor;
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let spec = PrinterSpec { files_root: dir.path().to_path_buf(), on_job: Arc::new(move |j| s2.lock().unwrap().push(j)) };
        let backend = super::super::rdp_drive::GuardedDriveBackend::new(&[]).with_printer(spec);
        let mut rdpdr = ironrdp_rdpdr::Rdpdr::new(Box::new(backend), "Remote".into()).with_printer_driver(
            PRINTER_DEVICE_ID,
            PRINTER_NAME.into(),
            MICROSOFT_PRINT_TO_PDF_DRIVER_NAME.into(),
        );
        let ioreq = |file_id: u32, major: u32, body: &[u8]| {
            let mut p = vec![0x72, 0x44, 0x52, 0x49]; // "rD" component, "IR" device I/O request
            for v in [PRINTER_DEVICE_ID, file_id, 1, major, 0] {
                p.extend_from_slice(&v.to_le_bytes());
            }
            p.extend_from_slice(body);
            p
        };
        // IRP_MJ_CREATE with an empty path.
        let mut create = Vec::new();
        create.extend_from_slice(&0x0012_0196u32.to_le_bytes()); // desired access
        create.extend_from_slice(&0u64.to_le_bytes()); // allocation size
        for v in [0u32, 7, 5, 0, 0] {
            create.extend_from_slice(&v.to_le_bytes()); // attributes, share, disposition, options, path length
        }
        let out = rdpdr.process(&ioreq(0, 0, &create)).unwrap();
        assert_eq!(out.len(), 1, "the create is answered");
        let pdf = b"%PDF-1.7\n% printed from Windows\n%%EOF\n";
        for chunk in pdf.chunks(16) {
            let mut w = Vec::new();
            w.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            w.extend_from_slice(&0u64.to_le_bytes());
            w.extend_from_slice(&[0u8; 20]);
            w.extend_from_slice(chunk);
            assert_eq!(rdpdr.process(&ioreq(1, 4, &w)).unwrap().len(), 1);
        }
        rdpdr.process(&ioreq(1, 2, &[0u8; 32])).unwrap(); // IRP_MJ_CLOSE
        let jobs = seen.lock().unwrap();
        assert_eq!(jobs.len(), 1, "{:?}", *jobs);
        assert_eq!(std::fs::read(dir.path().join(&jobs[0].rel_path)).unwrap(), pdf);
    }

    #[test]
    fn writes_to_an_unknown_job_are_refused() {
        let (mut s, _dir, _seen) = sink();
        assert!(!s.write_job(99, b"x"));
    }
}
