//! Start the real binary and talk MCP to it over stdio.

use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// A minimal `.hwpx`: two paragraphs and a 2×2 table.
fn write_hwpx(path: &Path) {
    let section = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" "#,
        r#"xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph">"#,
        r#"<hp:p><hp:run><hp:t>수사 결과 보고</hp:t></hp:run></hp:p>"#,
        r#"<hp:p><hp:run><hp:t>첫째 문단입니다. 둘째 문장도 있습니다.</hp:t></hp:run></hp:p>"#,
        r#"<hp:p><hp:run><hp:tbl colCnt="2">"#,
        r#"<hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>구분</hp:t></hp:run></hp:p></hp:subList></hp:tc>"#,
        r#"<hp:tc><hp:subList><hp:p><hp:run><hp:t>내용</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr>"#,
        r#"<hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>피의자</hp:t></hp:run></hp:p></hp:subList></hp:tc>"#,
        r#"<hp:tc><hp:subList><hp:p><hp:run><hp:t>홍길동</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr>"#,
        r#"</hp:tbl></hp:run></hp:p>"#,
        r#"</hs:sec>"#,
    );
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/hwp+zip").unwrap();
    zip.start_file("Contents/section0.xml", stored).unwrap();
    zip.write_all(section.as_bytes()).unwrap();
    zip.finish().unwrap();
}

struct Client {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Client {
    async fn start(root: &Path, max_chars: usize) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hwp2md-mcp"))
            .arg("--root")
            .arg(root)
            .arg("--max-chars")
            .arg(max_chars.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("start hwp2md-mcp");
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut client = Client {
            _child: child,
            stdin,
            lines,
            next_id: 0,
        };
        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "test", "version": "0" }
                }),
            )
            .await;
        client
            .send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        client
    }

    async fn send(&mut self, message: &Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let line = tokio::time::timeout(Duration::from_secs(20), self.lines.next_line())
                .await
                .expect("answer in time")
                .unwrap()
                .expect("server still running");
            let message: Value = serde_json::from_str(&line).expect("JSON line");
            if message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
        }
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        )
        .await
    }
}

fn text_of(result: &Value) -> String {
    result["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect()
}

fn folders() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    write_hwpx(&root.join("sub/보고서.hwpx"));
    std::fs::write(root.join("old.hwp"), b"not read").unwrap();
    write_hwpx(&dir.path().join("outside.hwpx"));
    (dir, root)
}

#[tokio::test]
async fn tools_are_listed_and_read_only() {
    let (_dir, root) = folders();
    let mut client = Client::start(&root, 40_000).await;
    let tools = client.request("tools/list", json!({})).await;
    let mut names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["hwpx_info", "hwpx_tables", "hwpx_to_markdown", "list_hwpx"]
    );
    for tool in tools["tools"].as_array().unwrap() {
        assert_eq!(tool["annotations"]["readOnlyHint"], json!(true), "{tool}");
    }
}

#[tokio::test]
async fn markdown_info_tables_and_listing() {
    let (_dir, root) = folders();
    let mut client = Client::start(&root, 40_000).await;

    let md = client
        .call("hwpx_to_markdown", json!({ "path": "sub/보고서.hwpx" }))
        .await;
    assert_ne!(md["isError"], json!(true), "{md}");
    let text = text_of(&md);
    assert!(text.contains("첫째 문단입니다."), "{text}");
    assert!(text.contains("| 피의자 | 홍길동 |"), "{text}");

    let info = client
        .call("hwpx_info", json!({ "path": "sub/보고서.hwpx" }))
        .await;
    let info = &info["structuredContent"];
    assert_eq!(
        (info["paragraphs"].as_u64(), info["tables"].as_u64()),
        (Some(6), Some(1))
    );

    let tables = client
        .call(
            "hwpx_tables",
            json!({ "path": "sub/보고서.hwpx", "index": 0 }),
        )
        .await;
    let t = &tables["structuredContent"];
    assert_eq!(t["total"], json!(1));
    assert_eq!(t["tables"][0]["cells"][1][1]["text"], json!("홍길동"));

    let list = client.call("list_hwpx", json!({})).await;
    let list = &list["structuredContent"];
    let files = list["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{list}");
    assert!(files[0]["path"].as_str().unwrap().ends_with("보고서.hwpx"));
    assert_eq!(list["hwp_skipped"], json!(1));
}

#[tokio::test]
async fn long_markdown_comes_in_pages() {
    let (_dir, root) = folders();
    let mut client = Client::start(&root, 25).await;
    let mut all = String::new();
    let mut offset = 0;
    for _ in 0..20 {
        let page = client
            .call(
                "hwpx_to_markdown",
                json!({ "path": "sub/보고서.hwpx", "offset": offset }),
            )
            .await;
        let text = text_of(&page);
        match text.split_once("\n\n[hwp2md: characters ") {
            Some((body, note)) => {
                all.push_str(body);
                let next = note.split("offset=").nth(1).unwrap();
                offset = next.split(' ').next().unwrap().parse().unwrap();
            }
            None => {
                all.push_str(&text);
                break;
            }
        }
    }
    assert!(
        all.contains("첫째 문단입니다.") && all.contains("홍길동"),
        "{all}"
    );
}

#[tokio::test]
async fn files_outside_the_root_and_hwp_files_are_refused() {
    let (dir, root) = folders();
    let mut client = Client::start(&root, 40_000).await;

    for path in [
        dir.path().join("outside.hwpx").display().to_string(),
        "../outside.hwpx".to_string(),
    ] {
        let r = client
            .call("hwpx_to_markdown", json!({ "path": path }))
            .await;
        assert_eq!(r["isError"], json!(true), "{r}");
        assert!(text_of(&r).contains("outside"), "{r}");
    }

    let r = client.call("hwpx_info", json!({ "path": "old.hwp" })).await;
    assert_eq!(r["isError"], json!(true), "{r}");
    assert!(text_of(&r).contains(".hwpx"), "{r}");
}
