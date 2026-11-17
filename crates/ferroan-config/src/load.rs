// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tranquillity's two configuration loaders.
//!
//! The region sequence is `ConfigurationLoader.LoadConfigSettings` in
//! `Source/OpenSim.Server.RegionServer/ConfigurationLoader.cs`. The grid
//! sequence is `GridServerConfigSource` in
//! `Source/OpenSim.Server.GridServer/GridServerConfigSource.cs`. Both at
//! Tranquillity `713857f10b8e91a639a967d4354a4682b080793c`.
//!
//! `config_dir` stands in for the process working directory (`Util.configDir`
//! is `"."`, and the grid loader's directory is `"."` as well). Environment
//! values come from the map passed in, not from the process environment, so a
//! fixture cannot capture the machine that generated it. An unset name still
//! becomes an empty string.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::error::ConfigError;
use crate::glob::glob;
use crate::parse::Config;

const MAX_SOURCES: usize = 10_000;

/// A missing `Include-*` file. The include value is the text from the INI, not
/// a resolved machine path. Tranquillity logs the absolute path and continues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadWarning {
    pub value: String,
}

/// A resolved configuration and the includes that were not on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub config: Config,
    pub warnings: Vec<LoadWarning>,
}

/// Arguments for the region loader.
#[derive(Debug, Clone)]
pub struct RegionLoad {
    pub config_dir: PathBuf,
    /// Command-line source merged last. `Startup` keys `inimaster`, `inifile`,
    /// `inifile2`, ... and `inidirectory` select files. Missing keys use
    /// Tranquillity's defaults.
    pub argv: Config,
    pub environment: BTreeMap<String, String>,
}

/// Arguments for the grid loader. Includes are merged after the base files,
/// so an included file overrides the master. The region loader queues includes
/// while it is still walking the base list, and its `inidirectory` is merged
/// after those includes.
#[derive(Debug, Clone)]
pub struct GridLoad {
    pub config_dir: PathBuf,
    /// `None` selects `OpenSim.Server.GridServer.ini`. A missing master is skipped.
    pub ini_master: Option<String>,
    pub ini_files: Vec<String>,
    /// `None` or empty skips the directory. Otherwise every `*.ini` in it is loaded.
    pub ini_directory: Option<String>,
    pub environment: BTreeMap<String, String>,
}

impl RegionLoad {
    /// Loads and substitutes the way `ConfigurationLoader` does.
    pub fn load(&self) -> Result<Resolved, ConfigError> {
        let mut master = self
            .argv
            .get("Startup", "inimaster")
            .unwrap_or("RegionServerDefaults.ini")
            .to_string();
        if master == "none" {
            master.clear();
        }

        let mut ini_names = vec![
            self.argv
                .get("Startup", "inifile")
                .unwrap_or("OpenSim.ini")
                .to_string(),
        ];
        for number in 2..1_000 {
            let key = format!("inifile{number}");
            let Some(name) = self.argv.get("Startup", &key) else {
                break;
            };
            ini_names.push(name.to_string());
        }

        let mut sources = Vec::new();
        if is_http(&master) {
            return Err(ConfigError::RemoteInclude { value: master });
        } else if !master.is_empty() {
            let path = full_path(&self.config_dir, &master);
            if !Path::new(&path).is_file() {
                return Err(ConfigError::MissingMaster { name: master });
            }
            push_unique(&mut sources, path)?;
        }

        for name in &ini_names {
            if is_http(name) {
                return Err(ConfigError::RemoteInclude {
                    value: name.clone(),
                });
            }
            let path = full_path(&self.config_dir, name);
            if !Path::new(&path).is_file() {
                let xml = full_path(&self.config_dir, "OpenSim.xml");
                if Path::new(&xml).is_file() {
                    return Err(ConfigError::XmlConfig);
                }
                continue;
            }
            push_unique(&mut sources, path)?;
        }

        let mut warnings = Vec::new();
        let mut config = Config::default();
        let mut index = 0;
        while index < sources.len() {
            let path = sources[index].clone();
            config.merge(&Config::parse_file(Path::new(&path))?);
            add_includes(&config, &mut sources, &self.config_dir, &mut warnings)?;
            index += 1;
        }

        let ini_directory = self
            .argv
            .get("Startup", "inidirectory")
            .unwrap_or("config")
            .to_string();
        let directory = located(&self.config_dir, &ini_directory);
        if directory.is_dir() {
            let mut override_sources = Vec::new();
            for path in list_ini(&directory, true)? {
                let full = get_full_path(&path.to_string_lossy());
                if !sources.iter().any(|item| item == &full) {
                    push_unique(&mut override_sources, full.clone())?;
                    push_unique(&mut sources, full)?;
                }
            }
            let mut override_config = Config::default();
            let mut override_index = 0;
            while override_index < override_sources.len() {
                let path = override_sources[override_index].clone();
                override_config.merge(&Config::parse_file(Path::new(&path))?);
                add_includes(
                    &override_config,
                    &mut override_sources,
                    &self.config_dir,
                    &mut warnings,
                )?;
                override_index += 1;
            }
            config.merge(&override_config);
        }

        if sources.is_empty() {
            return Err(ConfigError::NoConfig);
        }

        finish(&mut config, &self.environment, Some(&self.argv))?;
        Ok(Resolved {
            config,
            warnings: warning_list(warnings),
        })
    }
}

impl GridLoad {
    /// Loads and substitutes the way `GridServerConfigSource` does.
    pub fn load(&self) -> Result<Resolved, ConfigError> {
        let master = match &self.ini_master {
            Some(name) if !name.is_empty() => name.clone(),
            _ => "OpenSim.Server.GridServer.ini".to_string(),
        };
        let mut sources = Vec::new();
        let mut config = Config::default();
        let mut warnings = Vec::new();
        load_grid_source(&master, &self.config_dir, &mut config, &mut sources)?;
        for file in &self.ini_files {
            load_grid_source(file, &self.config_dir, &mut config, &mut sources)?;
        }
        if let Some(directory) = &self.ini_directory
            && !directory.is_empty()
        {
            let path = located(&self.config_dir, directory);
            if path.is_dir() {
                for file in list_ini(&path, false)? {
                    let stored = file.to_string_lossy().into_owned();
                    load_grid_source(&stored, &self.config_dir, &mut config, &mut sources)?;
                }
            }
        }

        let mut index = sources.len();
        loop {
            let before = sources.len();
            add_includes(&config, &mut sources, &self.config_dir, &mut warnings)?;
            if sources.len() == before {
                break;
            }
            while index < sources.len() {
                config.merge(&Config::parse_file(Path::new(&sources[index]))?);
                index += 1;
            }
        }

        finish(&mut config, &self.environment, None)?;
        Ok(Resolved {
            config,
            warnings: warning_list(warnings),
        })
    }
}

fn finish(
    config: &mut Config,
    environment: &BTreeMap<String, String>,
    argv: Option<&Config>,
) -> Result<(), ConfigError> {
    if config.apply_environment(environment) {
        config.expand()?;
    }
    if let Some(argv) = argv {
        config.merge(argv);
    }
    config.expand()
}

fn load_grid_source(
    stored: &str,
    config_dir: &Path,
    config: &mut Config,
    sources: &mut Vec<String>,
) -> Result<(), ConfigError> {
    if stored.is_empty() || sources.iter().any(|item| item == stored) {
        return Ok(());
    }
    if is_http(stored) {
        return Err(ConfigError::RemoteInclude {
            value: stored.to_string(),
        });
    }
    let path = located(config_dir, stored);
    if !path.is_file() {
        return Ok(());
    }
    config.merge(&Config::parse_file(&path)?);
    push_unique(sources, stored.to_string())
}

fn add_includes(
    config: &Config,
    sources: &mut Vec<String>,
    config_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<(), ConfigError> {
    let mut includes = Vec::new();
    for section in config.section_names() {
        for (key, value) in config.keys(section) {
            if key.starts_with("Include-") {
                includes.push(value.to_string());
            }
        }
    }
    for file in includes {
        if is_http(&file) {
            return Err(ConfigError::RemoteInclude { value: file });
        }
        let wildcard = file.contains(['*', '?']);
        let paths = resolve_include(config_dir, &file)?;
        if !wildcard && paths.is_empty() && !warnings.iter().any(|item| item == &file) {
            warnings.push(file);
            continue;
        }
        for path in paths {
            push_unique(sources, path)?;
        }
    }
    Ok(())
}

fn resolve_include(config_dir: &Path, file: &str) -> Result<Vec<String>, ConfigError> {
    let split_at = file.find(['*', '?']);
    let (prefix, suffix) = match split_at {
        Some(index) => (&file[..index], &file[index..]),
        None => (file, ""),
    };
    let combined = dotnet_combine(config_dir, prefix);
    let full = get_full_path(&combined);
    let pattern = format!("{full}{suffix}");
    let mut paths = Vec::new();
    for path in glob(&pattern)? {
        paths.push(path.to_string_lossy().into_owned());
    }
    paths.sort();
    Ok(paths)
}

fn list_ini(directory: &Path, ignore_case: bool) -> Result<Vec<PathBuf>, ConfigError> {
    let entries = std::fs::read_dir(directory).map_err(|error| ConfigError::Io {
        path: directory.display().to_string(),
        message: error.to_string(),
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ConfigError::Io {
            path: directory.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|item| item.to_str()) else {
            continue;
        };
        let matches = if ignore_case {
            name.len() >= 4 && name[name.len() - 4..].eq_ignore_ascii_case(".ini")
        } else {
            name.ends_with(".ini")
        };
        if matches {
            files.push(path);
        }
    }
    Ok(files)
}

fn finish_path_text(path: &str) -> String {
    get_full_path(path)
}

fn located(config_dir: &Path, name: &str) -> PathBuf {
    if Path::new(name).is_absolute() {
        PathBuf::from(name)
    } else {
        config_dir.join(name)
    }
}

fn full_path(config_dir: &Path, name: &str) -> String {
    finish_path_text(&dotnet_combine(config_dir, name))
}

fn dotnet_combine(base: &Path, extra: &str) -> String {
    if extra.is_empty() {
        return base.to_string_lossy().into_owned();
    }
    if Path::new(extra).is_absolute() {
        return extra.to_string();
    }
    let mut text = base.to_string_lossy().into_owned();
    let separator = std::path::MAIN_SEPARATOR;
    if !text.ends_with(separator) {
        text.push(separator);
    }
    text.push_str(extra);
    text
}

fn get_full_path(path: &str) -> String {
    let trailing = path.ends_with('/') || path.ends_with('\\');
    let absolute = std::path::absolute(Path::new(path)).unwrap_or_else(|_| PathBuf::from(path));
    // `absolute` leaves `.` and `..` in place. .NET GetFullPath removes them.
    let mut text = normalize_dots(&absolute).to_string_lossy().into_owned();
    if trailing && !text.ends_with(std::path::MAIN_SEPARATOR) {
        text.push(std::path::MAIN_SEPARATOR);
    }
    text
}

fn normalize_dots(path: &Path) -> PathBuf {
    let mut prefix: Option<std::ffi::OsString> = None;
    let mut rooted = false;
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = parts.pop();
            }
            Component::RootDir => rooted = true,
            Component::Prefix(prefix_component) => {
                prefix = Some(prefix_component.as_os_str().to_os_string());
                rooted = false;
                parts.clear();
            }
            Component::Normal(part) => parts.push(part.to_os_string()),
        }
    }
    // `PathBuf::push("\\")` on Windows replaces a drive prefix. The root
    // separator is appended as text so `C:\foo\..\bar` stays on `C:`.
    let mut out = if let Some(mut text) = prefix {
        if rooted {
            text.push(std::path::MAIN_SEPARATOR_STR);
        }
        PathBuf::from(text)
    } else if rooted {
        PathBuf::from(std::path::MAIN_SEPARATOR_STR)
    } else {
        PathBuf::new()
    };
    for part in parts {
        out.push(part);
    }
    out
}

fn is_http(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if rest.is_empty() || value.chars().any(char::is_whitespace) {
        return false;
    }
    scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
}

fn push_unique(sources: &mut Vec<String>, path: String) -> Result<(), ConfigError> {
    if sources.len() >= MAX_SOURCES {
        return Err(ConfigError::Limit {
            what: "include files",
        });
    }
    if !sources.iter().any(|item| item == &path) {
        sources.push(path);
    }
    Ok(())
}

fn warning_list(values: Vec<String>) -> Vec<LoadWarning> {
    values
        .into_iter()
        .map(|value| LoadWarning { value })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::normalize_dots;
    use std::path::{Path, PathBuf};

    #[cfg(unix)]
    #[test]
    fn normalize_removes_dot_and_dotdot() {
        assert_eq!(
            normalize_dots(Path::new("/tmp/ferroan/../keep/./file")),
            PathBuf::from("/tmp/keep/file")
        );
        assert_eq!(
            normalize_dots(Path::new("/../stay")),
            PathBuf::from("/stay")
        );
    }

    #[cfg(windows)]
    #[test]
    fn normalize_keeps_the_drive() {
        assert_eq!(
            normalize_dots(Path::new(r"C:\tmp\ferroan\..\keep\.\file")),
            PathBuf::from(r"C:\tmp\keep\file")
        );
        assert_eq!(
            normalize_dots(Path::new(r"C:\..\stay")),
            PathBuf::from(r"C:\stay")
        );
    }
}
