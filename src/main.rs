//! The Remote Files plugin of Ardoise: files on another machine, over
//! SSH. The system's `ssh` does the work (keys, the agent and
//! `~/.ssh/config` apply). A remote file is copied to a local folder, opened
//! there like any file, and sent back each time it is saved.

mod ssh;
mod text;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ardoise_plugin::protocol::{Context, InitializeParams, Settings, UiEvent};
use ardoise_plugin::ui::*;
use ardoise_plugin::{run, Host, Plugin};
use serde::{Deserialize, Serialize};

use ssh::{Entry, RemoteFile};
use text::t;

const PANEL: &str = "remote";
/// The largest file opened.
const MAX_BYTES: usize = 50 * 1024 * 1024;

/// A local copy and the remote file it goes back to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Link {
    host: String,
    path: String,
    /// When the remote file was last changed, as of the last copy.
    #[serde(default)]
    date: Option<u64>,
}

impl Link {
    /// `host:/path`, as the editor shows where the copy comes from.
    fn origin(&self) -> String {
        format!("{}:{}", self.host, self.path)
    }
}

/// What the plugin keeps: the machines it went to, the copies it made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    links: BTreeMap<PathBuf, Link>,
}

struct Remote {
    data_dir: PathBuf,
    kept: Kept,
    host: String,
    dir: String,
    entries: Vec<Entry>,
    error: Option<String>,
    french: bool,
    runtime: tokio::runtime::Runtime,
}

impl Remote {
    fn new() -> Self {
        Remote {
            data_dir: PathBuf::new(),
            kept: Kept::default(),
            host: String::new(),
            dir: ".".into(),
            entries: Vec::new(),
            error: None,
            french: false,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime"),
        }
    }

    fn kept_file(&self) -> PathBuf {
        self.data_dir.join("remote.json")
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.kept) {
            let _ = std::fs::write(self.kept_file(), json);
        }
    }

    fn remember_host(&mut self, host: &str) {
        self.kept.hosts.retain(|h| h != host);
        self.kept.hosts.insert(0, host.to_string());
        self.kept.hosts.truncate(10);
        self.save();
    }

    /// Lists the folder typed, on the machine typed; a whole name
    /// (`user@host:/path/file`) opens that file at once.
    /// Returns the file to open when a whole name was typed.
    fn connect(&mut self) -> Option<RemoteFile> {
        let typed = self.host.trim().to_string();
        if let Some(file) = ssh::parse(&typed) {
            return Some(file);
        }
        if !ssh::valid_host(&typed) {
            self.error = Some(t(self.french, "bad_host").to_string());
            return None;
        }
        let dir = if self.dir.trim().is_empty() {
            ".".to_string()
        } else {
            self.dir.trim().to_string()
        };
        match self.runtime.block_on(ssh::list(&typed, &dir)) {
            Ok((resolved, entries)) => {
                self.dir = resolved;
                self.entries = entries;
                self.error = None;
                self.remember_host(&typed);
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e);
            }
        }
        None
    }

    /// Copies the file here and opens the copy.
    fn open(&mut self, host: &Host, file: RemoteFile) {
        let (bytes, date) = match self.runtime.block_on(ssh::download(&file, MAX_BYTES)) {
            Ok(found) => found,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let Some(local) = ssh::local_copy(&self.data_dir.join("copies"), &file) else {
            return;
        };
        if let Some(parent) = local.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = write_private(&local, &bytes) {
            self.error = Some(e.to_string());
            return;
        }
        self.kept.links.insert(
            local.clone(),
            Link {
                host: file.host.clone(),
                path: file.path.clone(),
                date,
            },
        );
        self.remember_host(&file.host);
        self.error = None;
        if let Some(link) = self.kept.links.get(&local) {
            host.set_origin(&local, Some(link.origin()));
        }
        if let Err(e) = host.open_file(&local, None) {
            self.error = Some(e.message);
        }
    }

    /// A click in the list: a folder is entered, a file is copied here.
    fn pick(&mut self, host: &Host, name: &str) {
        let is_dir = name == ".." || self.entries.iter().any(|e| e.name == name && e.is_dir);
        if is_dir {
            self.dir = if name == ".." {
                ssh::parent(&self.dir)
            } else {
                ssh::join(&self.dir, name)
            };
            if let Some(file) = self.connect() {
                self.open(host, file);
            }
            return;
        }
        let file = RemoteFile {
            host: self.host.trim().to_string(),
            path: ssh::join(&self.dir, name),
        };
        self.open(host, file);
    }

    /// After a save: the copy goes back to the machine it came from, unless
    /// the file changed there since.
    fn send(&mut self, host: &Host, path: &Path) {
        let Some(link) = self.kept.links.get(path).cloned() else {
            return;
        };
        let f = self.french;
        host.notify(format!("{} {}…", t(f, "sending"), link.host));
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                host.error(e.to_string());
                return;
            }
        };
        let file = RemoteFile {
            host: link.host.clone(),
            path: link.path.clone(),
        };
        match self.runtime.block_on(ssh::upload(&file, bytes, link.date)) {
            Ok(date) => {
                if let Some(kept) = self.kept.links.get_mut(path) {
                    kept.date = date;
                }
                self.save();
                host.notify(format!("{} {}", t(f, "sent"), link.host));
            }
            Err(e) => host.error(format!("{}: {e}", t(f, "send_failed"))),
        }
    }

    fn view(&self) -> Node {
        let f = self.french;
        let mut rows = vec![
            input("host", &self.host, t(f, "host")).on_submit("go"),
            row([
                input("dir", &self.dir, t(f, "folder")).on_submit("go"),
                // Nowhere to go until a machine is named.
                button(t(f, "go"), "go")
                    .button_style(ButtonStyle::Primary)
                    .enabled(!self.host.trim().is_empty()),
            ])
            .spacing(6.0),
        ];
        if !self.kept.hosts.is_empty() {
            rows.push(select("recent", self.kept.hosts.clone(), None).placeholder(t(f, "recent")));
        }
        if let Some(e) = &self.error {
            rows.push(text(e.clone()).style(TextStyle::Error));
        }
        if !self.entries.is_empty() {
            let mut items = vec![item("..", "..").icon("\u{f062}")];
            items.extend(self.entries.iter().map(|e| {
                if e.is_dir {
                    item(e.name.clone(), format!("{}/", e.name)).icon("\u{f07b}")
                } else {
                    item(e.name.clone(), e.name.clone()).icon("\u{f15b}")
                }
            }));
            rows.push(scroll(list("entries", items)));
        } else if self.error.is_none() {
            rows.push(text(t(f, "hint")).style(TextStyle::Muted));
        }
        column(rows).spacing(8.0)
    }
}

/// Writes a file readable by its owner only (copies of private files).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

impl Plugin for Remote {
    fn initialize(&mut self, host: &Host, params: &InitializeParams) {
        self.data_dir = params.data_dir.clone();
        self.french = params.settings.locale.starts_with("fr");
        self.kept = std::fs::read_to_string(self.kept_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // The copies that are gone are forgotten.
        self.kept.links.retain(|local, _| local.is_file());
        self.host = self.kept.hosts.first().cloned().unwrap_or_default();
        // Each copy says where it comes from, in place of its folder.
        for (local, link) in &self.kept.links {
            host.set_origin(local, Some(link.origin()));
        }
    }

    fn command(&mut self, host: &Host, command: &str, _: &Context) {
        if command == "remote.open" {
            host.show_panel(PANEL);
            host.render(PANEL, self.view());
        }
    }

    fn panel_opened(&mut self, host: &Host, panel: &str) {
        if panel == PANEL {
            host.render(PANEL, self.view());
        }
    }

    fn ui_event(&mut self, host: &Host, event: &UiEvent) {
        match event.action.as_str() {
            "host" => {
                self.host = event.text().unwrap_or_default().to_string();
                return;
            }
            "dir" => {
                self.dir = event.text().unwrap_or_default().to_string();
                return;
            }
            "recent" => {
                self.host = event.text().unwrap_or_default().to_string();
                self.dir = ".".into();
                if let Some(file) = self.connect() {
                    self.open(host, file);
                }
            }
            "go" => {
                // The fields send their text with Enter.
                if let Some(text) = event.text() {
                    if ssh::parse(text).is_some() || ssh::valid_host(text) {
                        self.host = text.to_string();
                    }
                }
                if let Some(file) = self.connect() {
                    self.open(host, file);
                }
            }
            "entries" => {
                if let Some(name) = event.text().map(str::to_string) {
                    self.pick(host, &name);
                }
            }
            _ => return,
        }
        host.render(PANEL, self.view());
    }

    fn file_saved(&mut self, host: &Host, path: &Path) {
        self.send(host, path);
    }

    fn settings_changed(&mut self, host: &Host, settings: &Settings) {
        self.french = settings.locale.starts_with("fr");
        host.render(PANEL, self.view());
    }
}

fn main() -> std::io::Result<()> {
    run(Remote::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_is_off_until_a_machine_is_named() {
        let go_on = |r: &Remote| {
            let page = serde_json::to_string(&r.view()).unwrap();
            // A button that is off has `"enabled":false` after its action.
            !page.contains("\"action\":\"go\",\"style\":\"primary\",\"enabled\":false")
        };
        let mut r = Remote::new();
        assert!(!go_on(&r));
        r.host = "  ".into();
        assert!(!go_on(&r));
        r.host = "me@box".into();
        assert!(go_on(&r));
    }

    #[test]
    fn what_is_kept_survives_a_restart_and_gone_copies_are_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("a.txt");
        std::fs::write(&copy, "x").unwrap();
        let mut r = Remote::new();
        r.data_dir = dir.path().to_path_buf();
        r.kept.links.insert(
            copy.clone(),
            Link {
                host: "me@box".into(),
                path: "/a.txt".into(),
                date: Some(5),
            },
        );
        r.kept.links.insert(
            dir.path().join("gone"),
            Link {
                host: "me@box".into(),
                path: "/b".into(),
                date: None,
            },
        );
        r.remember_host("me@box");
        r.remember_host("other");
        r.remember_host("me@box");
        assert_eq!(
            r.kept.hosts,
            vec!["me@box".to_string(), "other".to_string()]
        );
        let mut again = Remote::new();
        again.kept =
            serde_json::from_str(&std::fs::read_to_string(r.kept_file()).unwrap()).unwrap();
        again.kept.links.retain(|local, _| local.is_file());
        assert_eq!(again.kept.links.len(), 1);
        assert_eq!(again.kept.links[&copy].date, Some(5));
    }

    #[test]
    fn each_copy_says_where_it_comes_from_once_the_plugin_starts() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        #[derive(Clone, Default)]
        struct Shared(Arc<Mutex<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("a.txt");
        std::fs::write(&copy, "x").unwrap();
        let mut r = Remote::new();
        r.data_dir = dir.path().to_path_buf();
        r.kept.links.insert(
            copy.clone(),
            Link {
                host: "me@box".into(),
                path: "/srv/a.txt".into(),
                date: None,
            },
        );
        r.remember_host("me@box");
        let out = Shared::default();
        let host = Host::to_writer(out.clone());
        let mut again = Remote::new();
        again.initialize(
            &host,
            &serde_json::from_value(serde_json::json!({
                "api": 1, "app_version": "1", "settings": {"locale": "en", "dark": false},
                "workspace": [], "plugin_dir": dir.path(), "data_dir": dir.path()
            }))
            .unwrap(),
        );
        let bytes = out.0.lock().unwrap().clone();
        let said = ardoise_plugin::protocol::read_message(&mut std::io::BufReader::new(&bytes[..]))
            .unwrap()
            .unwrap();
        let text = format!("{said:?}");
        assert!(text.contains("editor/setOrigin"), "{text}");
        assert!(text.contains("me@box:/srv/a.txt"), "{text}");
    }

    #[test]
    fn a_bad_machine_name_is_said_without_trying() {
        let mut r = Remote::new();
        r.host = "-oProxyCommand=evil".into();
        assert_eq!(r.connect(), None);
        assert_eq!(r.error.as_deref(), Some(t(false, "bad_host")));
        let page = serde_json::to_string(&r.view()).unwrap();
        assert!(page.contains("user@host"));
    }
}
