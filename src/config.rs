use std::env;
use std::path::{Path, PathBuf};

/// Parsed `~/.config/boltsnap/config.toml`. A missing file or an unset key leaves
/// the field `None`; defaults are applied by the `resolve_*` helpers.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Config {
    pub save_dir: Option<String>,
    pub editor: Option<String>,
}

impl Config {
    /// Parse config text. Unknown keys are ignored; a parse error logs and yields
    /// defaults (so a typo never bricks the daemon).
    pub fn parse(text: &str) -> Config {
        match toml::from_str::<toml::Value>(text) {
            Ok(v) => Config {
                save_dir: v.get("save_dir").and_then(|x| x.as_str()).map(String::from),
                editor: v.get("editor").and_then(|x| x.as_str()).map(String::from),
            },
            Err(e) => {
                eprintln!("boltsnap: ignoring malformed config: {e}");
                Config::default()
            }
        }
    }

    /// Load from `config_path()`; a missing file yields defaults.
    pub fn load() -> Config {
        match std::fs::read_to_string(config_path()) {
            Ok(s) => Config::parse(&s),
            Err(_) => Config::default(),
        }
    }
}

/// `$XDG_CONFIG_HOME/boltsnap/config.toml`, else `~/.config/boltsnap/config.toml`.
pub fn config_path() -> PathBuf {
    if let Some(x) = env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(x).join("boltsnap").join("config.toml")
    } else if let Some(home) = env::var_os("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("boltsnap")
            .join("config.toml")
    } else {
        PathBuf::from("boltsnap-config.toml")
    }
}

/// Expand a leading `~` (to `$HOME`) and `$VAR` / `${VAR}` tokens from the env.
/// Unset variables expand to empty.
pub fn expand_path(raw: &str) -> PathBuf {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut first = true;
    while let Some(c) = chars.next() {
        match c {
            '~' if first => match env::var_os("HOME") {
                Some(home) => out.push_str(&home.to_string_lossy()),
                None => out.push('~'),
            },
            '$' => {
                let braced = chars.peek() == Some(&'{');
                if braced {
                    chars.next();
                }
                let mut name = String::new();
                while let Some(&n) = chars.peek() {
                    let ok = if braced {
                        n != '}'
                    } else {
                        n.is_ascii_alphanumeric() || n == '_'
                    };
                    if ok {
                        name.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if braced && chars.peek() == Some(&'}') {
                    chars.next();
                }
                if let Some(val) = env::var_os(&name) {
                    out.push_str(&val.to_string_lossy());
                }
            }
            _ => out.push(c),
        }
        first = false;
    }
    PathBuf::from(out)
}

/// The shelf save directory: CLI flag > `$BOLTSNAP_SAVE_DIR` > config > `~/Bilder/boltsnap`.
pub fn resolve_save_dir(cli: Option<&Path>, cfg: &Config) -> PathBuf {
    if let Some(p) = cli {
        return p.to_path_buf();
    }
    if let Ok(e) = env::var("BOLTSNAP_SAVE_DIR") {
        if !e.is_empty() {
            return expand_path(&e);
        }
    }
    if let Some(s) = &cfg.save_dir {
        return expand_path(s);
    }
    default_save_dir()
}

fn default_save_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Bilder")
        .join("boltsnap")
}

/// The annotation editor command: CLI flag > `$BOLTSNAP_EDITOR` > config >
/// (`eddy` on PATH | `~/projects/eddy/build/eddy`). `None` if nothing is found.
pub fn resolve_editor(cli: Option<&str>, cfg: &Config) -> Option<String> {
    if let Some(c) = cli {
        return Some(c.to_string());
    }
    if let Ok(e) = env::var("BOLTSNAP_EDITOR") {
        if !e.is_empty() {
            return Some(expand_path(&e).to_string_lossy().into_owned());
        }
    }
    if let Some(e) = &cfg.editor {
        return Some(e.clone());
    }
    default_editor()
}

fn default_editor() -> Option<String> {
    if crate::paths::has_cmd("eddy") {
        return Some("eddy".to_string());
    }
    let built = env::var_os("HOME")
        .map(PathBuf::from)?
        .join("projects")
        .join("eddy")
        .join("build")
        .join("eddy");
    if built.is_file() {
        return Some(built.to_string_lossy().into_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_both_keys_and_ignores_unknown() {
        let c = Config::parse("save_dir = \"/tmp/shots\"\neditor = \"eddy\"\nbogus = 1\n");
        assert_eq!(c.save_dir.as_deref(), Some("/tmp/shots"));
        assert_eq!(c.editor.as_deref(), Some("eddy"));
    }

    #[test]
    fn parse_missing_keys_are_none() {
        let c = Config::parse("");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn parse_malformed_yields_defaults() {
        let c = Config::parse("save_dir = = =");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn expand_tilde_and_env() {
        // Do NOT mutate HOME (other modules' tests read it); assert against the
        // real value. BOLT_T is read by no other test, so setting it is race-free.
        let home = env::var("HOME").expect("HOME set in test env");
        unsafe {
            env::set_var("BOLT_T", "sub");
        }
        assert_eq!(expand_path("~/Bilder"), PathBuf::from(&home).join("Bilder"));
        assert_eq!(expand_path("/a/$BOLT_T/b"), PathBuf::from("/a/sub/b"));
        assert_eq!(expand_path("/a/${BOLT_T}x"), PathBuf::from("/a/subx"));
    }

    #[test]
    fn save_dir_precedence_flag_over_config() {
        let cfg = Config {
            save_dir: Some("/from/config".into()),
            editor: None,
        };
        let got = resolve_save_dir(Some(Path::new("/from/flag")), &cfg);
        assert_eq!(got, PathBuf::from("/from/flag"));
    }

    #[test]
    fn save_dir_falls_back_to_config_then_default() {
        // BOLTSNAP_SAVE_DIR is boltsnap-private; removing it is race-free. HOME is
        // left untouched and the expectation is derived from it.
        let home = env::var("HOME").expect("HOME set in test env");
        unsafe {
            env::remove_var("BOLTSNAP_SAVE_DIR");
        }
        let cfg = Config {
            save_dir: Some("~/cfgshots".into()),
            editor: None,
        };
        assert_eq!(
            resolve_save_dir(None, &cfg),
            PathBuf::from(&home).join("cfgshots")
        );
        assert_eq!(
            resolve_save_dir(None, &Config::default()),
            PathBuf::from(&home).join("Bilder").join("boltsnap")
        );
    }

    #[test]
    fn editor_precedence_flag_then_config() {
        let cfg = Config {
            save_dir: None,
            editor: Some("cfg-editor".into()),
        };
        assert_eq!(
            resolve_editor(Some("flag-editor"), &cfg),
            Some("flag-editor".to_string())
        );
        unsafe {
            env::remove_var("BOLTSNAP_EDITOR");
        }
        assert_eq!(resolve_editor(None, &cfg), Some("cfg-editor".to_string()));
    }
}
