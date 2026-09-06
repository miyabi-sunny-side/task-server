use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn retired_path_variables_do_not_change_startup_or_storage() {
    for configured in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let ledger = root.path().join(if configured {
            "configured-ledger"
        } else {
            "data/ledger"
        });
        let old_db = root.path().join("old.db");
        std::fs::write(&old_db, b"preserve migration source").unwrap();
        let projects = root.path().join("projects");
        std::fs::create_dir(&projects).unwrap();
        let mut expected_record = None;
        for (db_set, projects_set) in [(false, false), (true, false), (false, true), (true, true)] {
            let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = reservation.local_addr().unwrap();
            drop(reservation);
            let mut command = Command::new(env!("CARGO_BIN_EXE_task-server"));
            command
                .env_clear()
                .current_dir(root.path())
                .env("PORT", address.port().to_string())
                .env("LOG_LEVEL", "warn")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if configured {
                command.env("APP_DATA_DIR", &ledger);
            }
            if db_set {
                command.env("APP_DB_PATH", &old_db);
            }
            if projects_set {
                command.env("APP_PROJECTS_DIR", &projects);
            }
            let mut server = Server(command.spawn().unwrap());
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                assert!(
                    server.0.try_wait().unwrap().is_none(),
                    "server exited: configured={configured}, db={db_set}, projects={projects_set}"
                );
                if let Ok(stream) = TcpStream::connect(address) {
                    break stream;
                }
                assert!(Instant::now() < deadline, "server did not start");
                thread::sleep(Duration::from_millis(20));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .write_all(
                    b"GET /api/products HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            if expected_record.is_some() {
                assert!(response.contains("org/repo"), "{response}");
            }
            server.0.kill().unwrap();
            server.0.wait().unwrap();
            let mut stdout = String::new();
            server
                .0
                .stdout
                .take()
                .unwrap()
                .read_to_string(&mut stdout)
                .unwrap();
            let mut stderr = String::new();
            server
                .0
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            assert!(stdout.is_empty() && stderr.is_empty(), "{stdout}{stderr}");
            assert!(ledger.join("products").is_dir());
            let record = ledger.join("products/org%2Frepo.md");
            if let Some(expected) = &expected_record {
                assert_eq!(&std::fs::read(&record).unwrap(), expected);
            } else {
                let store = task_server::ledger::Store::open(&ledger).unwrap();
                store.put("products", "org/repo", serde_json::json!({"id":"org/repo","repository":"fixture","body":"preserved"})).unwrap();
                expected_record = Some(std::fs::read(&record).unwrap());
            }
            assert_eq!(
                std::fs::read(&old_db).unwrap(),
                b"preserve migration source"
            );
            assert!(std::fs::read_dir(&projects).unwrap().next().is_none());
            if configured {
                assert!(!root.path().join("data").exists());
            }
        }
    }
}
