pub mod auth;
pub mod config;
pub mod db;
pub mod protocols;
pub mod server;
pub mod transfer;

/// Initialize the process-wide Rustls crypto provider (aws-lc-rs or ring)
pub fn init_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::{Database, ConnectionRecord, User};
    use uuid::Uuid;

    #[test]
    fn test_granular_connection_rbac_and_isolation() {
        let db = Database::new_in_memory().expect("failed to create in-memory db");

        let admin_user = User {
            id: Uuid::new_v4().to_string(),
            username: "admin".into(),
            password_hash: "hash".into(),
            email: Some("admin@remote.local".into()),
            display_name: Some("Administrator".into()),
            role: "admin".into(),
            is_active: true,
            auth_provider: "local".into(),
            oidc_sub: None,
            created_at: "2026-09-01T00:00:00Z".into(),
            last_login: None,
            avatar_data: None,
        };

        let operator_a = User {
            id: Uuid::new_v4().to_string(),
            username: "alice".into(),
            password_hash: "hash".into(),
            email: Some("alice@domain.com".into()),
            display_name: Some("Alice".into()),
            role: "operator".into(),
            is_active: true,
            auth_provider: "local".into(),
            oidc_sub: None,
            created_at: "2026-09-01T00:00:00Z".into(),
            last_login: None,
            avatar_data: None,
        };

        let operator_b = User {
            id: Uuid::new_v4().to_string(),
            username: "bob".into(),
            password_hash: "hash".into(),
            email: Some("bob@domain.com".into()),
            display_name: Some("Bob".into()),
            role: "operator".into(),
            is_active: true,
            auth_provider: "local".into(),
            oidc_sub: None,
            created_at: "2026-09-01T00:00:00Z".into(),
            last_login: None,
            avatar_data: None,
        };

        // 1. Global shared connection created by Admin with View-Only policy
        let global_conn = ConnectionRecord {
            id: Uuid::new_v4().to_string(),
            name: "Global Web Monitor".into(),
            protocol: "vnc".into(),
            host: "10.0.0.1".into(),
            port: 5900,
            username: Some("viewer".into()),
            password_enc: None,
            private_key_enc: None,
            settings_json: "{}".into(),
            icon: None,
            tags: Some("monitor".into()),
            is_global: true,
            allow_clipboard: "host_to_remote".into(),
            allow_transfer: "disabled".into(),
            view_only: true,
            created_by: Some(admin_user.id.clone()),
            created_at: "2026-09-01T00:00:00Z".into(),
            updated_at: "2026-09-01T00:00:00Z".into(),
        };
        db.save_connection(&global_conn).expect("save global conn");

        // 2. Personal private connection created by Alice
        let alice_conn = ConnectionRecord {
            id: Uuid::new_v4().to_string(),
            name: "Alice Private Bastion".into(),
            protocol: "ssh".into(),
            host: "10.0.0.2".into(),
            port: 22,
            username: Some("alice".into()),
            password_enc: None,
            private_key_enc: None,
            settings_json: "{}".into(),
            icon: None,
            tags: Some("personal".into()),
            is_global: false,
            allow_clipboard: "bidirectional".into(),
            allow_transfer: "full".into(),
            view_only: false,
            created_by: Some(operator_a.id.clone()),
            created_at: "2026-09-01T00:00:00Z".into(),
            updated_at: "2026-09-01T00:00:00Z".into(),
        };
        db.save_connection(&alice_conn).expect("save alice conn");

        // Admin should see BOTH global and personal connections
        let admin_list = db.list_connections_for_user(&admin_user).expect("list admin");
        assert_eq!(admin_list.len(), 2);

        // Alice should see Global + her own Personal connection
        let alice_list = db.list_connections_for_user(&operator_a).expect("list alice");
        assert_eq!(alice_list.len(), 2);
        let alice_bastion = alice_list.iter().find(|c| c.id == alice_conn.id).expect("found alice conn");
        assert!(alice_bastion.user_permissions.can_edit);
        assert!(!alice_bastion.is_global);

        // Bob should ONLY see the Global connection (Alice's personal connection is isolated!)
        let bob_list = db.list_connections_for_user(&operator_b).expect("list bob");
        assert_eq!(bob_list.len(), 1);
        assert_eq!(bob_list[0].id, global_conn.id);
        assert!(!bob_list[0].user_permissions.can_edit); // Non-admin cannot edit global conn
        assert!(bob_list[0].is_global);
        assert!(bob_list[0].view_only);
        assert_eq!(bob_list[0].allow_clipboard, "host_to_remote");
        assert_eq!(bob_list[0].allow_transfer, "disabled");
    }

    #[test]
    fn test_user_disable_and_profile_updates() {
        let db = Database::new_in_memory().expect("failed to create in-memory db");

        let mut user = User {
            id: Uuid::new_v4().to_string(),
            username: "admin".into(),
            password_hash: "hash".into(),
            email: Some("admin@remote.local".into()),
            display_name: Some("Administrator".into()),
            role: "admin".into(),
            is_active: true,
            auth_provider: "local".into(),
            oidc_sub: None,
            created_at: "2026-09-01T00:00:00Z".into(),
            last_login: None,
            avatar_data: None,
        };
        db.create_user(&user).expect("create user");

        // 1. Update nickname and email
        user.display_name = Some("Woofson Supreme".into());
        user.email = Some("boss@boop.no".into());
        db.update_user(&user).expect("update user profile");

        let fetched = db.get_user_by_id(&user.id).expect("get user").expect("user exists");
        assert_eq!(fetched.display_name.as_deref(), Some("Woofson Supreme"));
        assert_eq!(fetched.email.as_deref(), Some("boss@boop.no"));
        assert!(fetched.is_active);

        // 2. Disable user account (including builtin admin)
        user.is_active = false;
        db.update_user(&user).expect("disable user");

        let disabled_user = db.get_user_by_id(&user.id).expect("get user").expect("user exists");
        assert!(!disabled_user.is_active);
    }

    #[test]
    fn test_telnet_naws_and_connection() {
        use protocols::telnet::naws_packet;

        // Verify NAWS packet RFC 1073 format: IAC SB NAWS <width_high> <width_low> <height_high> <height_low> IAC SE
        let packet = naws_packet(120, 32);
        assert_eq!(packet, vec![255, 250, 31, 0, 120, 0, 32, 255, 240]);

        // Verify zero fallback to 80x24 standard terminal size
        let default_packet = naws_packet(0, 0);
        assert_eq!(default_packet, vec![255, 250, 31, 0, 80, 0, 24, 255, 240]);

        // Verify Database supports Telnet connection records
        let db = Database::new_in_memory().expect("failed to create in-memory db");
        let conn = ConnectionRecord {
            id: Uuid::new_v4().to_string(),
            name: "BBS Retro Gateway".into(),
            protocol: "telnet".into(),
            host: "telehack.com".into(),
            port: 23,
            username: None,
            password_enc: None,
            private_key_enc: None,
            settings_json: "{}".into(),
            icon: None,
            tags: Some("telnet,bbs".into()),
            is_global: true,
            allow_clipboard: "bidirectional".into(),
            allow_transfer: "disabled".into(),
            view_only: false,
            created_by: None,
            created_at: "2026-09-01T00:00:00Z".into(),
            updated_at: "2026-09-01T00:00:00Z".into(),
        };
        db.save_connection(&conn).expect("save telnet conn");
        let loaded = db.get_connection_raw(&conn.id).expect("get conn").expect("found");
        assert_eq!(loaded.protocol, "telnet");
        assert_eq!(loaded.port, 23);
    }

    #[test]
    fn test_ssh_certificate_params_and_settings() {
        use protocols::ssh::SshConnectionParams;

        let cert_str = "ssh-ed25519-cert-v01@openssh.com AAAAIHNzaC1lZDI1NTE5LWNlcnQtdjAxQG9wZW5zc2guY29tAAAA... user@domain";
        let priv_key_str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA...\n-----END OPENSSH PRIVATE KEY-----";

        let params = SshConnectionParams {
            host: "10.0.0.5".into(),
            port: 22,
            username: "bolt".into(),
            password: None,
            private_key: Some(priv_key_str.into()),
            certificate: Some(cert_str.into()),
            passphrase: None,
        };

        assert_eq!(params.username, "bolt");
        assert_eq!(params.certificate.as_deref(), Some(cert_str));

        // Test settings_json parsing for SSH certificate
        let db = Database::new_in_memory().expect("failed to create in-memory db");
        let settings = serde_json::json!({
            "ssh_certificate": cert_str
        });

        let conn = ConnectionRecord {
            id: Uuid::new_v4().to_string(),
            name: "Vault CA Bastion".into(),
            protocol: "ssh".into(),
            host: "bastion.corp".into(),
            port: 2222,
            username: Some("deploy".into()),
            password_enc: None,
            private_key_enc: None,
            settings_json: settings.to_string(),
            icon: None,
            tags: Some("vault,ssh-ca".into()),
            is_global: true,
            allow_clipboard: "bidirectional".into(),
            allow_transfer: "full".into(),
            view_only: false,
            created_by: None,
            created_at: "2026-09-01T00:00:00Z".into(),
            updated_at: "2026-09-01T00:00:00Z".into(),
        };

        db.save_connection(&conn).expect("save ssh cert conn");
        let loaded = db.get_connection_raw(&conn.id).expect("get conn").expect("found");
        let parsed: serde_json::Value = serde_json::from_str(&loaded.settings_json).expect("valid json");
        assert_eq!(parsed["ssh_certificate"], cert_str);
    }

    #[test]
    fn test_vnc_des_auth_and_rfb_handshake() {
        use protocols::vnc::{
            connect_and_handshake_vnc, des_ecb_encrypt_block, map_browser_key_to_keysym,
            vnc_des_encrypt, VncConnectionParams,
        };
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // 1. Verify standard NIST FIPS 46-3 DES-ECB test vector
        let key = [0x13, 0x34, 0x57, 0x79, 0x9b, 0xbc, 0xdf, 0xf1];
        let plain = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        let cipher = des_ecb_encrypt_block(&key, &plain);
        assert_eq!(cipher, [0x85, 0xe8, 0x13, 0x54, 0x0f, 0x0a, 0xb4, 0x05]);

        // 2. Verify X11 keysym mapping for VNC keyboard input
        assert_eq!(map_browser_key_to_keysym(0x1C, 0, 0), Some(0xff0d)); // Enter
        assert_eq!(map_browser_key_to_keysym(0x0E, 0, 0), Some(0xff08)); // BackSpace
        assert_eq!(map_browser_key_to_keysym(0x48, 1, 0), Some(0xff52)); // ArrowUp
        assert_eq!(map_browser_key_to_keysym(0x1E, 0, b'a' as u16), Some(0x0061)); // 'a'
        assert_eq!(map_browser_key_to_keysym(0x27, 0, 'ø' as u16), Some(0x00f8)); // 'ø'

        // 3. Spin up mock RFB 3.8 server requiring VNC Authentication (type 2)
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock vnc");
        let port = listener.local_addr().unwrap().port();
        let challenge: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let expected_resp = vnc_des_encrypt(&challenge, "secret123");

        let server_thread = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept client");
            sock.write_all(b"RFB 003.008\n").unwrap();
            let mut c_ver = [0u8; 12];
            sock.read_exact(&mut c_ver).unwrap();
            assert_eq!(&c_ver, b"RFB 003.008\n");

            // Offer security types [1, 2]
            sock.write_all(&[2, 1, 2]).unwrap();
            let mut chosen = [0u8; 1];
            sock.read_exact(&mut chosen).unwrap();
            assert_eq!(chosen[0], 2); // Client selects VNC Auth (2) because password is provided

            sock.write_all(&challenge).unwrap();
            let mut resp = [0u8; 16];
            sock.read_exact(&mut resp).unwrap();
            assert_eq!(resp, expected_resp);

            // SecurityResult = 0 (OK)
            sock.write_all(&0u32.to_be_bytes()).unwrap();

            // Read ClientInit (shared = 1)
            let mut client_init = [0u8; 1];
            sock.read_exact(&mut client_init).unwrap();
            assert_eq!(client_init[0], 1);

            // Send ServerInit (1920x1080, name = "Worktop VNC")
            let mut srv_init = [0u8; 24];
            srv_init[0..2].copy_from_slice(&1920u16.to_be_bytes());
            srv_init[2..4].copy_from_slice(&1080u16.to_be_bytes());
            let name = b"Worktop VNC";
            srv_init[20..24].copy_from_slice(&(name.len() as u32).to_be_bytes());
            sock.write_all(&srv_init).unwrap();
            sock.write_all(name).unwrap();

            // Read SetPixelFormat (20 bytes), SetEncodings (4 + 9*4 = 40 bytes), and FramebufferUpdateRequest (10 bytes)
            let mut post_init = [0u8; 70];
            sock.read_exact(&mut post_init).unwrap();
            assert_eq!(post_init[0], 0); // SetPixelFormat
            assert_eq!(post_init[20], 2); // SetEncodings
            assert_eq!(post_init[60], 3); // FramebufferUpdateRequest
        });

        let params = VncConnectionParams {
            host: "127.0.0.1".into(),
            port,
            username: None,
            password: Some("secret123".into()),
        };

        let (_stream, info) = connect_and_handshake_vnc(&params).expect("vnc handshake ok");
        assert_eq!(info.width, 1920);
        assert_eq!(info.height, 1080);
        assert_eq!(info.name, "Worktop VNC");
        server_thread.join().unwrap();
    }
}
