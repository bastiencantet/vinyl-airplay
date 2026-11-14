//! Real native launcher smoke test: no capture, receiver session or Python runtime.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Launcher {
    child: Child,
    state: PathBuf,
}
impl Drop for Launcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.state);
    }
}
fn http(port: u16, method: &str, path: &str, headers: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}
#[test]
fn real_native_web_launcher_embeds_ui_and_stays_off_on_fresh_state() {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let state = std::env::temp_dir().join(format!("vw-{}", uuid::Uuid::new_v4().simple()));
    let child = Command::new(env!("CARGO_BIN_EXE_vinyl-web"))
        .args([
            "--port",
            &port.to_string(),
            "--sender",
            env!("CARGO_BIN_EXE_vinyl-airplay"),
            "--state",
        ])
        .arg(&state)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut launcher = Launcher { child, state };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        assert!(
            launcher.child.try_wait().unwrap().is_none(),
            "launcher exited during startup"
        );
        assert!(Instant::now() < deadline, "launcher did not become ready");
        std::thread::sleep(Duration::from_millis(20));
    }
    let response = http(port, "GET", "/api/state", "", "");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let state: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(state["running"], false);
    assert_eq!(state["config"]["enabled"], false);
    let page = http(port, "GET", "/", "", "");
    assert!(page.contains("Vinyl AirPlay"));
    assert!(page.contains("content-security-policy:"));
    let denied = http(
        port,
        "POST",
        "/api/config",
        "Content-Type: application/json\r\nOrigin: http://attacker.example\r\n",
        "{}",
    );
    assert!(denied.starts_with("HTTP/1.1 403"));
    let mut config = state["config"].clone();
    config["gain_db"] = 3.into();
    let saved = http(
        port,
        "POST",
        "/api/config",
        "Content-Type: application/json\r\nX-Vinyl-Control: 1\r\n",
        &config.to_string(),
    );
    assert!(saved.starts_with("HTTP/1.1 200"), "{saved}");
    let disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(launcher.state.join("control/config.json")).unwrap())
            .unwrap();
    assert_eq!(disk, config);
    #[cfg(unix)]
    {
        let status = Command::new("/bin/kill")
            .args(["-TERM", &launcher.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(exit) = launcher.child.try_wait().unwrap() {
                assert!(exit.success(), "{exit}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "SIGTERM did not stop the native launcher"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
