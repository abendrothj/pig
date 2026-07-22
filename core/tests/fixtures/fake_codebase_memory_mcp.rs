//! A fake `codebase-memory-mcp` executable, built as a real binary and located at test
//! time via `CARGO_BIN_EXE_fake_codebase_memory_mcp`, so `CodebaseMemoryCliProvider`'s
//! integration tests exercise a real child process (spawn, pipes, exit status, timeout)
//! without depending on the real tool being installed. Behavior is selected via a
//! `fake_mode` key in the JSON args written to stdin, mirroring the real tool's
//! `cli <tool>` (stdin-JSON) contract.

use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--version") {
        println!("fake_codebase_memory_mcp 0.0.0");
        return;
    }

    let tool = args.get(2).cloned().unwrap_or_default();

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let parsed: serde_json::Value = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let mode = parsed
        .get("fake_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("success");

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    match mode {
        "success" => {
            let _ = writeln!(out, "{}", serde_json::json!({"tool": tool, "ok": true}));
        }
        "malformed" => {
            let _ = writeln!(out, "not valid json {{");
        }
        "timeout" => {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        "error" => {
            eprintln!("simulated provider error");
            std::process::exit(1);
        }
        "large" => {
            let chunk = vec![b'x'; 65536];
            for _ in 0..32 {
                let _ = out.write_all(&chunk);
            }
            let _ = out.flush();
        }
        _ => {
            let _ = writeln!(out, "{{}}");
        }
    }
}
