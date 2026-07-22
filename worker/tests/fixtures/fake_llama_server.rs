//! A fake `llama-server` executable fixture, built as a real binary and located at
//! test time via `CARGO_BIN_EXE_fake_llama_server`, so `LlamaCppBackend`'s integration
//! tests exercise a real child process (spawn, HTTP, exit status) without requiring a
//! real llama.cpp install or GGUF model. Behavior is selected via `FAKE_LLAMA_MODE`.
//! Deliberately a hand-rolled HTTP/1.1 server (no tokio/axum) - a fixture process
//! should be as simple as possible to reason about independently of the code under
//! test.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help") {
        println!(
            "--ctx-size --threads --threads-batch --gpu-layers --batch-size \
             --ubatch-size --flash-attn --no-mmap --mlock --host --port -m"
        );
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("version: 9999 (fake-fixture)");
        return;
    }
    if args.iter().any(|a| a == "--list-devices") {
        println!("Available devices:\n  CPU (fake)");
        return;
    }

    let mode = std::env::var("FAKE_LLAMA_MODE").unwrap_or_else(|_| "success".to_string());

    if mode == "startup_failure" {
        eprintln!("fatal: failed to load model (simulated)");
        std::process::exit(1);
    }
    if mode == "readiness_delay" {
        std::thread::sleep(std::time::Duration::from_millis(400));
    }

    let host = arg_value(&args, "--host").unwrap_or_else(|| "127.0.0.1".to_string());
    let port: u16 = arg_value(&args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);

    let listener = TcpListener::bind((host.as_str(), port)).expect("bind fake llama-server");

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        handle(&mut stream, &mode);
        if mode == "crash_after_first_request" {
            std::process::exit(1);
        }
    }
}

fn handle(stream: &mut TcpStream, mode: &str) {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).unwrap_or(0);
    let request = String::from_utf8_lossy(&buf[..n]);

    if request.starts_with("GET /health") {
        write_response(stream, 200, "application/json", r#"{"status":"ok"}"#);
        return;
    }

    match mode {
        "malformed_json" => {
            write_sse(stream, "data: not valid json {\n\ndata: [DONE]\n\n");
        }
        "request_failure" => {
            write_response(
                stream,
                500,
                "application/json",
                r#"{"error":"simulated failure"}"#,
            );
        }
        "excessive_stderr" => {
            eprintln!("{}", "x".repeat(1000));
            write_sse(stream, success_stream());
        }
        _ => {
            write_sse(stream, success_stream());
        }
    }
}

fn success_stream() -> &'static str {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":null},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\" (thinking)\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":\"stop\"}],\"timings\":{\"prompt_ms\":1.0,\"predicted_ms\":2.0,\"prompt_per_second\":100.0,\"predicted_per_second\":100.0,\"prompt_n\":3,\"predicted_n\":3}}\n\n",
        "data: [DONE]\n\n"
    )
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
}

fn write_sse(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 X\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
        body
    );
    let _ = stream.write_all(response.as_bytes());
}
