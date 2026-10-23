use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use tempfile::NamedTempFile;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sdk {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Mapping::is_empty")]
    pub plugs: Mapping,
    #[serde(default, skip_serializing_if = "Mapping::is_empty")]
    pub slots: Mapping,
}

impl Sdk {
    pub fn new(name: String, channel: Option<String>) -> Self {
        Self {
            name,
            channel,
            plugs: Mapping::new(),
            slots: Mapping::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub plug: String,
    pub slot: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub name: String,
    pub base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sdks: Vec<Sdk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connections: Option<Vec<Connection>>,
    #[serde(default, skip_serializing_if = "Mapping::is_empty")]
    pub actions: Mapping,
}

fn matches(pattern: &str, value: &str) -> bool {
    Regex::new(pattern)
        .expect("static regular expression")
        .is_match(value)
}

fn identifier(name: &str) -> bool {
    matches(r"^[a-z](-?[a-z0-9])*$", name)
}

fn sdk_name(name: &str) -> bool {
    let underlying = name
        .strip_prefix("try-")
        .or_else(|| name.strip_prefix("project-"))
        .unwrap_or(name);
    underlying.len() <= 40
        && underlying != "agent"
        && !underlying.starts_with("try-")
        && !underlying.starts_with("project-")
        && underlying.bytes().any(|c| c.is_ascii_lowercase())
        && matches(r"^[a-z0-9](-?[a-z0-9])*$", underlying)
}

fn channel_valid(channel: &str) -> bool {
    let risks = ["stable", "candidate", "beta", "edge"];
    let parts: Vec<_> = channel.split('/').collect();
    let track = |s: &str| s.len() <= 28 && matches(r"^[a-zA-Z0-9]([_.-]?[a-zA-Z0-9])*$", s);
    let branch = |s: &str| {
        s.len() <= 128
            && !risks.contains(&s)
            && matches(r"^[a-zA-Z0-9][a-zA-Z0-9.-]*[a-zA-Z0-9]$", s)
    };
    match parts.as_slice() {
        [one] => risks.contains(one) || track(one),
        [a, b] => {
            (risks.contains(a) && branch(b))
                || (!risks.contains(a) && track(a) && risks.contains(b))
        }
        [a, b, c] => !risks.contains(a) && track(a) && risks.contains(b) && branch(c),
        _ => false,
    }
}

impl Definition {
    fn directory_name(directory: &Path) -> String {
        let basename = directory.file_name().and_then(|name| name.to_str()).unwrap_or("");
        let mut name = String::new();
        for character in basename.chars() {
            if character.is_ascii_alphanumeric() {
                if name.is_empty() && !character.is_ascii_alphabetic() {
                    continue;
                }
                name.push(character.to_ascii_lowercase());
            } else if !name.is_empty() && !name.ends_with('-') {
                name.push('-');
            }
            if name.len() == 40 {
                break;
            }
        }
        let name = name.trim_end_matches('-');
        if name.is_empty() { "my-workshop".into() } else { name.into() }
    }

    pub fn empty() -> Self {
        Self {
            name: "my-workshop".into(),
            base: "ubuntu@24.04".into(),
            runtime: None,
            sdks: vec![],
            connections: None,
            actions: Mapping::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let definition: Self = serde_yaml::from_str(text).context("Invalid workshop YAML")?;
        definition.validate()?;
        Ok(definition)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            identifier(&self.name) && self.name.len() <= 40,
            "Workshop name must start with a lowercase letter, contain lowercase letters/digits joined by single hyphens, and be at most 40 characters"
        );
        ensure!(
            ["ubuntu@22.04", "ubuntu@24.04", "ubuntu@26.04"].contains(&self.base.as_str()),
            "Unsupported base: {}",
            self.base
        );
        if let Some(runtime) = &self.runtime {
            ensure!(
                ["lxd-container", "lxd-vm"].contains(&runtime.as_str()),
                "Unsupported runtime: {runtime}"
            );
        }
        let mut names = HashSet::from(["system", "sketch"]);
        let mut explicit_names = HashSet::new();
        let mut bound = HashSet::new();
        for sdk in &self.sdks {
            ensure!(sdk_name(&sdk.name), "Invalid SDK name: {}", sdk.name);
            ensure!(
                explicit_names.insert(sdk.name.as_str()),
                "Duplicate SDK: {}",
                sdk.name
            );
            names.insert(sdk.name.as_str());
            if let Some(channel) = &sdk.channel {
                ensure!(channel_valid(channel), "Invalid channel: {channel}");
            }
            for (kind, endpoints) in [("plug", &sdk.plugs), ("slot", &sdk.slots)] {
                for (key, value) in endpoints {
                    let key = key
                        .as_str()
                        .context("Plug and slot names must be strings")?;
                    ensure!(identifier(key), "Invalid {kind} name: {key}");
                    ensure!(
                        value.is_null() || value.is_string() || value.is_mapping(),
                        "Invalid {kind} definition: {key}"
                    );
                    if kind == "plug"
                        && let Some(binding) = value.get("bind")
                    {
                        ensure!(
                            value.as_mapping().is_some_and(|m| m.len() == 1),
                            "Bound plug {key} cannot have other attributes"
                        );
                        let target = binding.as_str().context("Plug bind must be a string")?;
                        bound.insert(format!("{}:{key}", sdk.name));
                        ensure!(
                            !target.starts_with("system:") && !target.starts_with(':'),
                            "Cannot bind to system plug"
                        );
                    }
                }
            }
        }
        let check_ref = |reference: &str| -> Result<()> {
            let (sdk, endpoint) = reference
                .split_once(':')
                .context("Endpoint must be <sdk>:<endpoint>")?;
            ensure!(
                identifier(endpoint) && (sdk.is_empty() || names.contains(sdk)),
                "Unknown SDK or invalid endpoint: {reference}"
            );
            Ok(())
        };
        for sdk in &self.sdks {
            for value in sdk.plugs.values() {
                if let Some(target) = value.get("bind").and_then(Value::as_str) {
                    check_ref(target)?;
                    ensure!(
                        !bound.contains(target),
                        "Cannot bind to an already bound plug: {target}"
                    );
                }
            }
        }
        for connection in self.connections.iter().flatten() {
            check_ref(&connection.plug)?;
            check_ref(&connection.slot)?;
            ensure!(
                !bound.contains(&connection.plug),
                "Bound plug cannot also be connected: {}",
                connection.plug
            );
        }
        for (name, script) in &self.actions {
            ensure!(
                name.as_str().is_some_and(identifier) && script.is_string(),
                "Actions must have valid names and string scripts"
            );
        }
        Ok(())
    }

    pub fn yaml(&self) -> Result<String> {
        self.validate()?;
        serde_yaml::to_string(self).context("Cannot serialize definition")
    }

    pub fn add_sdk(&mut self, sdk: Sdk) -> Result<()> {
        let mut next = self.clone();
        next.sdks.push(sdk);
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn remove_sdk(&mut self, index: usize) -> Result<()> {
        ensure!(index < self.sdks.len(), "Select an SDK first");
        let mut next = self.clone();
        next.sdks.remove(index);
        next.validate()
            .context("SDK is still referenced; edit connections/bindings first")?;
        *self = next;
        Ok(())
    }
}

pub struct Document {
    pub path: PathBuf,
    pub definition: Definition,
    saved: Definition,
    original: Option<Vec<u8>>,
}

impl Document {
    pub fn open(path: PathBuf) -> Result<Self> {
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        Self::check_path(&path)?;
        let original = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("Cannot read definition"),
        };
        let definition = match &original {
            Some(bytes) => {
                Definition::parse(std::str::from_utf8(bytes).context("Definition is not UTF-8")?)?
            }
            None => {
                let mut definition = Definition::empty();
                definition.name = Definition::directory_name(&std::env::current_dir()?);
                definition
                    .sdks
                    .push(Sdk::new("uv".into(), Some("latest/stable".into())));
                if path
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|n| n == ".workshop")
                {
                    definition.name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .context("Definition filename is not UTF-8")?
                        .to_owned();
                }
                definition
            }
        };
        let document = Self {
            path,
            saved: definition.clone(),
            definition,
            original,
        };
        document.validate()?;
        Ok(document)
    }

    fn check_path(path: &Path) -> Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Definition must be a regular file, not a symlink"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Cannot inspect definition path"),
        }
        Ok(())
    }

    pub fn dirty(&self) -> bool {
        self.original.is_none() || self.definition != self.saved
    }

    pub fn validate(&self) -> Result<()> {
        self.definition.validate()?;
        if self
            .path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == ".workshop")
        {
            ensure!(
                self.path.file_stem().and_then(|s| s.to_str()) == Some(&self.definition.name),
                "Definitions in .workshop/ must match their filename"
            );
        }
        Ok(())
    }

    pub fn save(&mut self) -> Result<()> {
        self.validate()?;
        Self::check_path(&self.path)?;
        let current = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("Cannot read definition before saving"),
        };
        ensure!(
            current == self.original,
            "Definition changed on disk; refusing to overwrite it. Reopen the TUI to load those changes"
        );
        let yaml = self.definition.yaml()?;
        let parent = self
            .path
            .parent()
            .context("Definition has no parent directory")?;
        let mut temp = NamedTempFile::new_in(parent).context("Cannot create save file")?;
        if current.is_some() {
            temp.as_file()
                .set_permissions(fs::metadata(&self.path)?.permissions())?;
        }
        temp.write_all(yaml.as_bytes())?;
        temp.as_file().sync_all()?;
        if current.is_none() {
            temp.persist_noclobber(&self.path)
                .context("Cannot create definition without overwriting")?;
        } else {
            temp.persist(&self.path)
                .context("Cannot replace definition")?;
        }
        fs::File::open(parent)?.sync_all()?;
        self.original = Some(yaml.into_bytes());
        self.saved = self.definition.clone();
        Ok(())
    }
}

fn editor_command(
    visual: Option<OsString>,
    editor: Option<OsString>,
    path: Option<OsString>,
) -> Result<Command> {
    for (name, value) in [("VISUAL", visual), ("EDITOR", editor)] {
        if let Some(value) = value {
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("{name} is not valid UTF-8"))?;
            if value.trim().is_empty() {
                continue;
            }
            let args = shell_words::split(&value)
                .with_context(|| format!("Cannot parse {name} editor command"))?;
            let Some((program, args)) = args.split_first() else {
                bail!("{name} editor command is empty")
            };
            ensure!(!program.is_empty(), "{name} editor program is empty");
            let mut command = Command::new(program);
            command.args(args);
            return Ok(command);
        }
    }
    const EDITORS: [&str; 8] = [
        "nano", "pico", "micro", "nvim", "vim", "vi", "emacs", "editor",
    ];
    if let Some(path) = path {
        let directories: Vec<_> = std::env::split_paths(&path).collect();
        for editor in EDITORS {
            for directory in &directories {
                let candidate = directory.join(editor);
                let metadata = match fs::metadata(&candidate) {
                    Ok(metadata) => metadata,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("Cannot inspect editor {}", candidate.display())
                        });
                    }
                };
                if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                    let candidate = if candidate.is_absolute() {
                        candidate
                    } else {
                        std::env::current_dir()?.join(candidate)
                    };
                    return Ok(Command::new(candidate));
                }
            }
        }
    }
    bail!(
        "Cannot find an editor on PATH (tried {}). Install nano or set VISUAL or EDITOR (for example EDITOR='nano')",
        EDITORS.join(", ")
    )
}

pub fn snap_root() -> Option<PathBuf> {
    if std::env::var_os("SNAP_NAME").as_deref() == Some(std::ffi::OsStr::new("workshop-config")) {
        std::env::var_os("SNAP").map(PathBuf::from)
    } else {
        None
    }
}

fn external_editor_command(
    snap: Option<&Path>,
    visual: Option<OsString>,
    editor: Option<OsString>,
    path: Option<OsString>,
) -> Result<Command> {
    if let Some(root) = snap {
        let mut command = Command::new(root.join("usr/bin/nano"));
        command.arg("--ignorercfiles");
        return Ok(command);
    }
    editor_command(visual, editor, path)
}

pub fn edit_external(definition: &Definition) -> Result<Definition> {
    let mut command = external_editor_command(
        snap_root().as_deref(),
        std::env::var_os("VISUAL"),
        std::env::var_os("EDITOR"),
        std::env::var_os("PATH"),
    )?;
    let mut temp = tempfile::Builder::new().suffix(".yaml").tempfile()?;
    temp.write_all(definition.yaml()?.as_bytes())?;
    let status = command
        .arg(temp.path())
        .status()
        .with_context(|| format!("Cannot start editor {:?}", command.get_program()))?;
    ensure!(
        status.success(),
        "Editor exited with {status}; changes were not applied"
    );
    let text = fs::read_to_string(temp.path()).context("Cannot read edited YAML")?;
    match Definition::parse(&text) {
        Ok(definition) => Ok(definition),
        Err(error) => {
            let (_, path) = temp
                .keep()
                .context("Cannot preserve invalid editor draft")?;
            Err(error).with_context(|| {
                format!(
                    "Edited YAML was not applied. Recover your draft at {}",
                    path.display()
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_names_are_valid_workshop_names() {
        for (directory, expected) in [
            ("/home/user/project", "project"),
            ("/home/user/My_Project", "my-project"),
            ("/home/user/123--My Project!", "my-project"),
            ("/", "my-workshop"),
            ("/home/user/123", "my-workshop"),
        ] {
            assert_eq!(Definition::directory_name(Path::new(directory)), expected);
        }
        let name = Definition::directory_name(Path::new(&"a".repeat(60)));
        assert_eq!(name.len(), 40);
        let mut definition = Definition::empty();
        definition.name = name;
        definition.validate().unwrap();
    }

    #[test]
    fn new_documents_use_current_directory_but_keep_named_definition_and_saved_names() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workshop.yaml");
        let mut document = Document::open(path.clone()).unwrap();
        assert_eq!(document.definition.name, Definition::directory_name(&std::env::current_dir().unwrap()));
        document.definition.name = "saved-name".into();
        document.save().unwrap();
        assert_eq!(Document::open(path).unwrap().definition.name, "saved-name");
        let named = directory.path().join(".workshop/dev.yaml");
        fs::create_dir(named.parent().unwrap()).unwrap();
        assert_eq!(Document::open(named).unwrap().definition.name, "dev");
    }

    fn mock_editor(directory: &Path, name: &str, mode: u32) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn strict_snap_uses_bundled_nano_instead_of_host_editor_settings() {
        let root = Path::new("/snap/workshop-config/current");
        let command = external_editor_command(
            Some(root),
            Some("host-editor --wait".into()),
            Some("other-host-editor".into()),
            Some("/host/bin".into()),
        )
        .unwrap();
        assert_eq!(command.get_program(), root.join("usr/bin/nano"));
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["--ignorercfiles"]);
        let source_command =
            external_editor_command(None, Some("host-editor --wait".into()), None, None).unwrap();
        assert_eq!(source_command.get_program(), "host-editor");
    }

    #[test]
    fn explicit_editors_keep_precedence_and_quoted_arguments() {
        let command = editor_command(
            Some("'my visual' --wait 'two words'".into()),
            Some("nano".into()),
            None,
        )
        .unwrap();
        assert_eq!(command.get_program(), "my visual");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--wait", "two words"]
        );
        let command = editor_command(Some("  ".into()), Some("pico -w".into()), None).unwrap();
        assert_eq!(command.get_program(), "pico");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["-w"]);
        assert!(editor_command(Some("'unclosed".into()), Some("nano".into()), None).is_err());
        assert!(editor_command(Some("''".into()), None, None).is_err());
        let command =
            editor_command(Some("missing-editor".into()), Some("nano".into()), None).unwrap();
        assert_eq!(command.get_program(), "missing-editor");
    }

    #[test]
    fn discovers_nano_before_pico_even_when_later_on_path() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        mock_editor(first.path(), "pico", 0o755);
        let nano = mock_editor(second.path(), "nano", 0o755);
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let command = editor_command(None, None, Some(path)).unwrap();
        assert_eq!(command.get_program(), nano);
    }

    #[test]
    fn discovers_pico_then_other_editors_and_skips_nonexecutables() {
        let dir = tempfile::tempdir().unwrap();
        mock_editor(dir.path(), "nano", 0o644);
        let pico = mock_editor(dir.path(), "pico", 0o755);
        let vim = mock_editor(dir.path(), "vim", 0o755);
        let path = Some(dir.path().as_os_str().to_os_string());
        assert_eq!(
            editor_command(None, None, path.clone())
                .unwrap()
                .get_program(),
            pico
        );
        fs::remove_file(pico).unwrap();
        assert_eq!(
            editor_command(Some("".into()), Some("".into()), path.clone())
                .unwrap()
                .get_program(),
            vim
        );
        fs::remove_file(vim).unwrap();
        fs::create_dir(dir.path().join("micro")).unwrap();
        let error = editor_command(None, None, path).unwrap_err().to_string();
        assert!(error.contains("Install nano or set VISUAL or EDITOR"));
        assert!(editor_command(None, None, None).is_err());
    }

    #[test]
    fn discovers_symlinked_editors_and_runs_the_resolved_command() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let editor = mock_editor(dir.path(), "real-editor", 0o755);
        let nano = dir.path().join("nano");
        symlink(editor, &nano).unwrap();
        let mut command =
            editor_command(None, None, Some(dir.path().as_os_str().to_os_string())).unwrap();
        assert_eq!(command.get_program(), nano);
        assert!(command.status().unwrap().success());
    }

    #[test]
    fn existing_definition_round_trips_all_semantics() {
        let yaml = r#"name: design
base: ubuntu@24.04
sdks:
- name: system
  plugs:
    penpot:
      interface: tunnel
      endpoint: 4400
- name: project-tools
  slots:
    cache: {interface: mount}
- name: rust
  channel: latest/stable
actions:
  hello: |
    printf '%s\n' hello
connections:
- plug: rust:ssh-agent
  slot: system:ssh-agent
"#;
        let definition = Definition::parse(yaml).unwrap();
        assert_eq!(
            Definition::parse(&definition.yaml().unwrap()).unwrap(),
            definition
        );
        assert!(definition.sdks.iter().any(|s| !s.plugs.is_empty()));
        assert!(!definition.actions.is_empty());
    }

    #[test]
    fn rejects_invalid_definitions_and_preserves_failed_mutations() {
        for yaml in [
            "name: Bad\nbase: ubuntu@24.04",
            "name: okay\nbase: ubuntu@20.04",
            "name: okay\nbase: ubuntu@24.04\nunknown: true",
        ] {
            assert!(Definition::parse(yaml).is_err());
        }
        let mut d = Definition::empty();
        d.add_sdk(Sdk::new("rust".into(), Some("latest/stable".into())))
            .unwrap();
        let before = d.clone();
        assert!(d.add_sdk(Sdk::new("rust".into(), None)).is_err());
        assert_eq!(before, d);
        d.connections = Some(vec![Connection {
            plug: "rust:ssh-agent".into(),
            slot: "system:ssh-agent".into(),
        }]);
        assert!(d.remove_sdk(0).is_err());
        assert_eq!(d.sdks.len(), 1);
    }

    #[test]
    fn checks_names_channels_and_bindings() {
        for name in ["agent", "try-agent", "project-try-rust", "rust--sdk", "123"] {
            assert!(!sdk_name(name), "{name}");
        }
        for channel in [
            "latest/stable",
            "stable",
            "1.2",
            "edge/my-branch",
            "latest/beta/testing",
        ] {
            assert!(channel_valid(channel), "{channel}");
        }
        for channel in ["", "latest/unknown", "a/b/c/d", "latest/stable/edge"] {
            assert!(!channel_valid(channel), "{channel}");
        }
        assert!(Definition::parse("name: okay\nbase: ubuntu@24.04\nsdks:\n- name: rust\n  plugs:\n    cache:\n      bind: system:cache").is_err());
    }

    #[test]
    fn safe_atomic_save_and_conflict_detection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workshop.yaml");
        let mut doc = Document::open(path.clone()).unwrap();
        assert!(doc.dirty());
        doc.save().unwrap();
        assert!(!doc.dirty());
        let original = fs::read(&path).unwrap();
        doc.definition.name = "new-name".into();
        fs::write(&path, "# concurrent edit\n").unwrap();
        assert!(doc.save().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "# concurrent edit\n");
        fs::write(&path, original).unwrap();
        doc.save().unwrap();
        assert_eq!(Document::open(path).unwrap().definition.name, "new-name");
    }

    #[test]
    fn workshop_directory_requires_matching_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".workshop");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("test.yaml"), "name: test\nbase: ubuntu@24.04\n").unwrap();
        let mut doc = Document::open(path.join("test.yaml")).unwrap();
        doc.definition.name = "other".into();
        assert!(doc.save().is_err());
        let fresh = Document::open(path.join("new-workshop.yaml")).unwrap();
        assert_eq!(fresh.definition.name, "new-workshop");
    }

    #[test]
    fn rejects_symlinks_and_preserves_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workshop.yaml");
        let mut doc = Document::open(path.clone()).unwrap();
        doc.save().unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        doc.definition.name = "updated".into();
        doc.save().unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let link = dir.path().join("link.yaml");
        symlink(&path, &link).unwrap();
        assert!(Document::open(link).is_err());
    }
}
