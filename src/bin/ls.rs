use lsp_server::{Connection, Message, Request, RequestId, Response};
use lsp_types::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

fn main() -> anyhow::Result<()> {
    let (connection, io_threads) = Connection::stdio();

    let server_caps = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_formatting_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        ..Default::default()
    })
    .unwrap();

    let init_params = connection.initialize(server_caps)?;
    let _params: InitializeParams = serde_json::from_value(init_params)?;

    // nvim logs stderr as ERROR, so only print when explicitly debugging.
    if std::env::var_os("SPICE_NETLIST_LS_LOG").is_some() {
        eprintln!("spice-netlist-ls: started (dialect auto-detect, formatter + definition)");
    }

    // Text of open buffers, keyed by URI. LSP didOpen/didChange keep this
    // in sync; formatting/definition respond against the in-memory text,
    // falling back to disk only for unopened files.
    let mut docs: HashMap<Uri, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }
                handle_request(&connection, req, &docs)?;
            }
            Message::Response(_) => {}
            Message::Notification(notif) => match notif.method.as_str() {
                "textDocument/didOpen" => {
                    let params: DidOpenTextDocumentParams = serde_json::from_value(notif.params)?;
                    let uri = params.text_document.uri;
                    let text = params.text_document.text;
                    docs.insert(uri.clone(), text.clone());
                    publish_diagnostics(&connection, &uri, &text)?;
                }
                "textDocument/didChange" => {
                    let params: DidChangeTextDocumentParams = serde_json::from_value(notif.params)?;
                    // full sync — the last content change carries the whole document
                    if let Some(text) = params.content_changes.last().map(|c| c.text.clone()) {
                        docs.insert(params.text_document.uri.clone(), text.clone());
                        publish_diagnostics(&connection, &params.text_document.uri, &text)?;
                    }
                }
                "textDocument/didClose" => {
                    let params: DidCloseTextDocumentParams = serde_json::from_value(notif.params)?;
                    docs.remove(&params.text_document.uri);
                    publish_diagnostics(&connection, &params.text_document.uri, "")?;
                }
                _ => {}
            },
        }
    }

    // The writer thread owns a receiver on connection's channel; if `connection`
    // is still alive here the writer never sees channel-close and join() hangs.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

/// In-memory text for `uri` if the client has the document open, else the
/// on-disk contents. Likes an empty string when neither exists.
fn text_for(uri: &Uri, docs: &HashMap<Uri, String>) -> String {
    docs.get(uri)
        .cloned()
        .or_else(|| std::fs::read_to_string(uri.path().as_str()).ok())
        .unwrap_or_default()
}

fn publish_diagnostics(connection: &Connection, uri: &Uri, text: &str) -> anyhow::Result<()> {
    let dialect = spice_netlist_ls::get_dialect(spice_netlist_ls::detect_dialect(text));
    let path = PathBuf::from(uri.path().as_str());
    // `external_subckts` is section-aware: if the file contains
    // `simulator lang=` directives, includes are walked per section under
    // that section's dialect and the results unioned.
    let external = spice_netlist_ls::linter::external_subckts(&path, &dialect);
    let opts = spice_netlist_ls::linter::LintOptions { external_subckts: external };
    // `lint_str` is section-aware: it segments the file and offsets
    // diagnostic line numbers to global coordinates internally.
    let diags = spice_netlist_ls::linter::lint_str(text, &dialect, &opts)
        .into_iter()
        .map(|d| Diagnostic {
            range: Range {
                start: Position { line: d.range.start_line, character: d.range.start_col },
                end: Position { line: d.range.end_line, character: d.range.end_col },
            },
            severity: Some(match d.severity {
                spice_netlist_ls::linter::Severity::Error => DiagnosticSeverity::ERROR,
                spice_netlist_ls::linter::Severity::Warning => DiagnosticSeverity::WARNING,
            }),
            code: Some(NumberOrString::String(d.code.to_string())),
            source: Some("spice-netlist-ls".to_string()),
            message: d.message,
            ..Default::default()
        })
        .collect();
    connection.sender.send(Message::Notification(lsp_server::Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::to_value(PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics: diags,
            version: None,
        })?,
    }))?;
    Ok(())
}

fn handle_request(
    connection: &Connection,
    req: Request,
    docs: &HashMap<Uri, String>,
) -> anyhow::Result<()> {
    match req.method.as_str() {
        "textDocument/formatting" => {
            let (id, params): (RequestId, DocumentFormattingParams) =
                (req.id, serde_json::from_value(req.params)?);
            let uri = params.text_document.uri;
            let text = text_for(&uri, docs);
            let opts = spice_netlist_ls::config::format_options_for(
                Some(&PathBuf::from(uri.path().as_str())),
                None,
                spice_netlist_ls::detect_dialect(&text),
            );
            let formatted = spice_netlist_ls::format_str(&text, &opts);
            let edits = if text == formatted {
                Vec::new()
            } else {
                vec![TextEdit {
                    range: Range {
                        start: Position { line: 0, character: 0 },
                        end: Position { line: u32::MAX, character: 0 },
                    },
                    new_text: formatted,
                }]
            };
            let resp = Response {
                id,
                result: Some(serde_json::to_value(edits)?),
                error: None,
            };
            connection.sender.send(Message::Response(resp))?;
        }
        "textDocument/definition" => {
            let (id, params): (RequestId, GotoDefinitionParams) =
                (req.id, serde_json::from_value(req.params)?);
            let uri = params.text_document_position_params.text_document.uri;
            let path = PathBuf::from(uri.path().as_str());
            let text = text_for(&uri, docs);
            let line = params.text_document_position_params.position.line as usize;
            let fallback = spice_netlist_ls::detect_dialect(&text);
            let secs = spice_netlist_ls::segments::segments(&text, fallback);
            // Find which section `line` falls in; parse *that section* under
            // its dialect to extract the X-instance ref name. The def search
            // then covers the whole file (all sections) + includes.
            let location = spice_netlist_ls::segments::section_index_at_line(&secs, line)
                .and_then(|i| {
                    let sec = &secs[i];
                    let sub_dialect = spice_netlist_ls::get_dialect(sec.dialect);
                    let within = line - sec.line_offset;
                    spice_netlist_ls::parser::subckt_ref_at_line(
                        sec.body,
                        within,
                        sub_dialect.as_ref(),
                    )
                })
                .and_then(|name| {
                    let mut visited = HashSet::new();
                    find_subckt_def(&path, &name, &secs, &mut visited)
                });
            let resp = Response {
                id,
                result: Some(serde_json::to_value(location)?),
                error: None,
            };
            connection.sender.send(Message::Response(resp))?;
        }
        _ => {
            let resp = Response {
                id: req.id,
                result: Some(serde_json::Value::Null),
                error: None,
            };
            connection.sender.send(Message::Response(resp))?;
        }
    }
    Ok(())
}

/// Find a subckt definition in `text`, following `.include`/`.inc`/`.lib`
/// directives transitively (relative paths resolve against the including
/// file's directory; cycles guarded via `visited`).
///
/// Section-aware: `secs` is the segmentation of the file (the caller segments
/// once at the root; recursive calls for includes segment their own text).
/// Each section's defs are scanned under that section's dialect; each
/// section's includes are walked under that section's dialect. The def line
/// is reported in global (whole-file) coordinates.
fn find_subckt_def(
    path: &Path,
    name: &str,
    secs: &[spice_netlist_ls::segments::Section<'_>],
    visited: &mut HashSet<PathBuf>,
) -> Option<Location> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return None;
    }
    // Search every section's defs under its own dialect. The def line is
    // offset to global coordinates.
    for sec in secs {
        let sub_dialect = spice_netlist_ls::get_dialect(sec.dialect);
        if let Some((_, line)) = spice_netlist_ls::parser::subckt_definitions(sec.body, sub_dialect.as_ref())
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            return Some(Location {
                uri: Uri::from_str(&format!("file://{}", path.display())).ok()?,
                range: Range::new(
                    Position::new((sec.line_offset + line) as u32, 0),
                    Position::new((sec.line_offset + line) as u32, 0),
                ),
            });
        }
    }
    // Walk includes per section under that section's dialect.
    for sec in secs {
        let sub_dialect = spice_netlist_ls::get_dialect(sec.dialect);
        for inc in spice_netlist_ls::parser::include_paths(sec.body, sub_dialect.as_ref()) {
            let inc_path = if Path::new(&inc).is_absolute() {
                PathBuf::from(&inc)
            } else {
                path.parent()?.join(&inc)
            };
            if let Ok(inc_text) = std::fs::read_to_string(&inc_path) {
                let inc_fallback = sub_dialect.kind();
                let inc_secs = spice_netlist_ls::segments::segments(&inc_text, inc_fallback);
                if let Some(loc) = find_subckt_def(&inc_path, name, &inc_secs, visited) {
                    return Some(loc);
                }
            }
        }
    }
    None
}
