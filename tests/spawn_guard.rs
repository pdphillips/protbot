use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

#[test]
fn missing_pass_entry_exits_without_prompting() {
    let dir = std::env::temp_dir().join(format!("protbot-spawn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let pass = bin.join("pass");
    std::fs::write(&pass, "#!/bin/sh\necho 'pass would prompt' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&pass, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spawn = format!("{}/scripts/spawn.sh", env!("CARGO_MANIFEST_DIR"));
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());

    let missing_token = Command::new("bash")
        .arg(&spawn)
        .env("PATH", &path)
        .env_remove("PROTON_PASS_AGENT_TOKEN")
        .env_remove("PROTON_BRIDGE_PASSWORD")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!missing_token.status.success());
    let err = String::from_utf8_lossy(&missing_token.stderr);
    assert!(err.contains("protbot/agent-token"), "{err}");
    assert!(err.contains("will not prompt"), "{err}");
    assert!(!err.contains("pass would prompt"), "{err}");

    let missing_bridge = Command::new("bash")
        .arg(&spawn)
        .env("PATH", &path)
        .env("PROTON_PASS_AGENT_TOKEN", "already-set")
        .env_remove("PROTON_BRIDGE_PASSWORD")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!missing_bridge.status.success());
    let err = String::from_utf8_lossy(&missing_bridge.stderr);
    assert!(err.contains("protbot/bridge-password"), "{err}");
    assert!(err.contains("will not prompt"), "{err}");
    assert!(!err.contains("pass would prompt"), "{err}");
    assert!(!err.contains("already-set"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
