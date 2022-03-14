use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cargo::CargoResult;
use indexmap::IndexSet;

use super::manifest::str_or_1_len_table;

/// A dependency handled by Cargo
///
/// `None` means the field will be blank in TOML
#[derive(Debug, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub struct Dependency {
    /// The name of the dependency (as it is set in its `Cargo.toml` and known to crates.io)
    pub name: String,
    /// Whether the dependency is opted-in with a feature flag
    pub optional: Option<bool>,

    /// List of features to add (or None to keep features unchanged).
    pub features: Option<IndexSet<String>>,
    /// Whether default features are enabled
    pub default_features: Option<bool>,

    /// Where the dependency comes from
    pub source: Option<Source>,
    /// Non-default registry
    pub registry: Option<String>,

    /// If the dependency is renamed, this is the new name for the dependency
    /// as a string.  None if it is not renamed.
    pub rename: Option<String>,

    /// Features that are exposed by the dependency
    pub available_features: BTreeMap<String, Vec<String>>,
}

impl Dependency {
    /// Create a new dependency with a name
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            optional: None,
            features: None,
            default_features: None,
            source: None,
            registry: None,
            rename: None,
            available_features: Default::default(),
        }
    }

    /// Set dependency to a given version
    pub fn set_source(mut self, source: impl Into<Source>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Remove the existing version requirement
    pub fn clear_version(mut self) -> Self {
        match &mut self.source {
            Some(Source::Registry(_)) => {
                self.source = None;
            }
            Some(Source::Path(path)) => {
                path.version = None;
            }
            Some(Source::Git(_)) => {}
            None => {}
        }
        self
    }

    /// Set the available features of the dependency to a given vec
    pub fn set_available_features(
        mut self,
        available_features: BTreeMap<String, Vec<String>>,
    ) -> Self {
        self.available_features = available_features;
        self
    }

    /// Populate from cargo
    pub fn set_available_features_from_cargo(
        mut self,
        available_features: &cargo::core::summary::FeatureMap,
    ) -> Dependency {
        self.available_features = available_features
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    v.iter()
                        .filter_map(|v| match v {
                            cargo::core::summary::FeatureValue::Feature(f) => {
                                Some(f.as_str().to_owned())
                            }
                            cargo::core::summary::FeatureValue::Dep { .. }
                            | cargo::core::summary::FeatureValue::DepFeature { .. } => None,
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        self
    }

    /// Set whether the dependency is optional
    #[allow(dead_code)]
    pub fn set_optional(mut self, opt: bool) -> Self {
        self.optional = Some(opt);
        self
    }

    /// Set features as an array of string (does some basic parsing)
    #[allow(dead_code)]
    pub fn set_features(mut self, features: IndexSet<String>) -> Self {
        self.features = Some(features);
        self
    }
    /// Set features as an array of string (does some basic parsing)
    pub fn extend_features(mut self, features: impl IntoIterator<Item = String>) -> Self {
        self.features
            .get_or_insert_with(Default::default)
            .extend(features);
        self
    }

    /// Set the value of default-features for the dependency
    #[allow(dead_code)]
    pub fn set_default_features(mut self, default_features: bool) -> Self {
        self.default_features = Some(default_features);
        self
    }

    /// Set the alias for the dependency
    pub fn set_rename(mut self, rename: &str) -> Self {
        self.rename = Some(rename.into());
        self
    }

    /// Set the value of registry for the dependency
    pub fn set_registry(mut self, registry: impl Into<String>) -> Self {
        self.registry = Some(registry.into());
        self
    }

    /// Get the dependency source
    pub fn source(&self) -> Option<&Source> {
        self.source.as_ref()
    }

    /// Get version of dependency
    pub fn version(&self) -> Option<&str> {
        match self.source()? {
            Source::Registry(src) => Some(src.version.as_str()),
            Source::Path(src) => src.version.as_deref(),
            Source::Git(_) => None,
        }
    }

    /// Get registry of the dependency
    pub fn registry(&self) -> Option<&str> {
        self.registry.as_deref()
    }

    /// Get the alias for the dependency (if any)
    pub fn rename(&self) -> Option<&str> {
        self.rename.as_deref()
    }

    /// Whether default features are activated
    pub fn default_features(&self) -> Option<bool> {
        self.default_features
    }

    /// Get whether the dep is optional
    pub fn optional(&self) -> Option<bool> {
        self.optional
    }

    /// Get the SourceID for this dependency
    pub fn source_id(&self, config: &cargo::Config) -> CargoResult<cargo::core::SourceId> {
        match &self.source.as_ref() {
            Some(Source::Registry(_)) | None => {
                if let Some(r) = self.registry() {
                    let source_id = cargo::core::SourceId::alt_registry(config, r)?;
                    Ok(source_id)
                } else {
                    let source_id = cargo::core::SourceId::crates_io(config)?;
                    Ok(source_id)
                }
            }
            Some(Source::Path(source)) => {
                let source_id = cargo::core::SourceId::for_path(&source.path)?;
                Ok(source_id)
            }
            Some(Source::Git(source)) => {
                let git_url = source.git.parse::<url::Url>()?;
                let git_ref = source.git_ref();
                let source_id = cargo::core::SourceId::for_git(&git_url, git_ref)?;
                Ok(source_id)
            }
        }
    }

    /// Query to find this dependency
    pub fn query(
        &self,
        config: &cargo::Config,
    ) -> CargoResult<cargo::core::dependency::Dependency> {
        let source_id = self.source_id(config)?;
        cargo::core::dependency::Dependency::parse(self.name.as_str(), self.version(), source_id)
    }
}

impl Dependency {
    /// Create a dependency from a TOML table entry
    pub fn from_toml(crate_root: &Path, key: &str, item: &toml_edit::Item) -> CargoResult<Self> {
        if let Some(version) = item.as_str() {
            let dep = Self::new(key).set_source(RegistrySource::new(version));
            Ok(dep)
        } else if let Some(table) = item.as_table_like() {
            let (name, rename) = if let Some(value) = table.get("package") {
                (
                    value
                        .as_str()
                        .ok_or_else(|| invalid_type(key, "package", value.type_name(), "string"))?
                        .to_owned(),
                    Some(key.to_owned()),
                )
            } else {
                (key.to_owned(), None)
            };

            let source: Source =
                if let Some(git) = table.get("git") {
                    let mut src = GitSource::new(
                        git.as_str()
                            .ok_or_else(|| invalid_type(key, "git", git.type_name(), "string"))?,
                    );
                    if let Some(value) = table.get("branch") {
                        src = src.set_branch(value.as_str().ok_or_else(|| {
                            invalid_type(key, "branch", value.type_name(), "string")
                        })?);
                    }
                    if let Some(value) = table.get("tag") {
                        src = src.set_tag(value.as_str().ok_or_else(|| {
                            invalid_type(key, "tag", value.type_name(), "string")
                        })?);
                    }
                    if let Some(value) = table.get("rev") {
                        src = src.set_rev(value.as_str().ok_or_else(|| {
                            invalid_type(key, "rev", value.type_name(), "string")
                        })?);
                    }
                    src.into()
                } else if let Some(path) = table.get("path") {
                    let path = crate_root
                        .join(path.as_str().ok_or_else(|| {
                            invalid_type(key, "path", path.type_name(), "string")
                        })?);
                    let mut src = PathSource::new(path);
                    if let Some(value) = table.get("version") {
                        src = src.set_version(value.as_str().ok_or_else(|| {
                            invalid_type(key, "version", value.type_name(), "string")
                        })?);
                    }
                    src.into()
                } else if let Some(version) = table.get("version") {
                    let src = RegistrySource::new(version.as_str().ok_or_else(|| {
                        invalid_type(key, "version", version.type_name(), "string")
                    })?);
                    src.into()
                } else {
                    anyhow::bail!("Unrecognized dependency source for `{key}`");
                };
            let registry = if let Some(value) = table.get("registry") {
                Some(
                    value
                        .as_str()
                        .ok_or_else(|| invalid_type(key, "registry", value.type_name(), "string"))?
                        .to_owned(),
                )
            } else {
                None
            };

            let default_features = table.get("default-features").and_then(|v| v.as_bool());

            let features = if let Some(value) = table.get("features") {
                Some(
                    value
                        .as_array()
                        .ok_or_else(|| invalid_type(key, "features", value.type_name(), "array"))?
                        .iter()
                        .map(|v| {
                            v.as_str().map(|s| s.to_owned()).ok_or_else(|| {
                                invalid_type(key, "features", v.type_name(), "string")
                            })
                        })
                        .collect::<CargoResult<IndexSet<String>>>()?,
                )
            } else {
                None
            };

            let available_features = BTreeMap::default();

            let optional = table.get("optional").and_then(|v| v.as_bool());

            let dep = Self {
                name,
                rename,
                source: Some(source),
                registry,
                default_features,
                features,
                available_features,
                optional,
            };
            Ok(dep)
        } else {
            anyhow::bail!("Unrecognized` dependency entry format for `{key}");
        }
    }

    /// Get the dependency name as defined in the manifest,
    /// that is, either the alias (rename field if Some),
    /// or the official package name (name field).
    pub fn toml_key(&self) -> &str {
        self.rename().unwrap_or(&self.name)
    }

    /// Convert dependency to TOML
    ///
    /// Returns a tuple with the dependency's name and either the version as a `String`
    /// or the path/git repository as an `InlineTable`.
    /// (If the dependency is set as `optional` or `default-features` is set to `false`,
    /// an `InlineTable` is returned in any case.)
    ///
    /// # Panic
    ///
    /// Panics if the path is relative
    pub fn to_toml(&self, crate_root: &Path) -> toml_edit::Item {
        assert!(
            crate_root.is_absolute(),
            "Absolute path needed, got: {}",
            crate_root.display()
        );
        let table: toml_edit::Item = match (
            self.optional.unwrap_or(false),
            self.features.as_ref(),
            self.default_features.unwrap_or(true),
            self.source.as_ref(),
            self.registry.as_ref(),
            self.rename.as_ref(),
        ) {
            // Extra short when version flag only
            (
                false,
                None,
                true,
                Some(Source::Registry(RegistrySource { version: v })),
                None,
                None,
            ) => toml_edit::value(v),
            // Other cases are represented as an inline table
            (_, _, _, _, _, _) => {
                let mut table = toml_edit::InlineTable::default();

                match &self.source {
                    Some(Source::Registry(src)) => {
                        table.insert("version", src.version.as_str().into());
                    }
                    Some(Source::Path(src)) => {
                        let relpath = path_field(crate_root, &src.path);
                        if let Some(r) = src.version.as_deref() {
                            table.insert("version", r.into());
                        }
                        table.insert("path", relpath.into());
                    }
                    Some(Source::Git(src)) => {
                        table.insert("git", src.git.as_str().into());
                        if let Some(branch) = src.branch.as_deref() {
                            table.insert("branch", branch.into());
                        }
                        if let Some(tag) = src.tag.as_deref() {
                            table.insert("tag", tag.into());
                        }
                        if let Some(rev) = src.rev.as_deref() {
                            table.insert("rev", rev.into());
                        }
                    }
                    None => {}
                }
                if table.contains_key("version") {
                    if let Some(r) = self.registry.as_deref() {
                        table.insert("registry", r.into());
                    }
                }

                if self.rename.is_some() {
                    table.insert("package", self.name.as_str().into());
                }
                if let Some(v) = self.default_features {
                    table.insert("default-features", v.into());
                }
                if let Some(features) = self.features.as_ref() {
                    let features: toml_edit::Value = features.iter().cloned().collect();
                    table.insert("features", features);
                }
                if let Some(v) = self.optional {
                    table.insert("optional", v.into());
                }

                toml_edit::value(toml_edit::Value::InlineTable(table))
            }
        };

        table
    }

    /// Modify existing entry to match this dependency
    pub fn update_toml(&self, crate_root: &Path, item: &mut toml_edit::Item) {
        if str_or_1_len_table(item) {
            // Nothing to preserve
            *item = self.to_toml(crate_root);
        } else if !is_package_eq(item, &self.name, self.rename.as_deref()) {
            // No existing keys are relevant when the package changes
            *item = self.to_toml(crate_root);
        } else if let Some(table) = item.as_table_like_mut() {
            match &self.source {
                Some(Source::Registry(src)) => {
                    table.insert("version", toml_edit::value(src.version.as_str()));
                    for key in ["path", "git", "branch", "tag", "rev"] {
                        table.remove(key);
                    }
                }
                Some(Source::Path(src)) => {
                    let relpath = path_field(crate_root, &src.path);
                    if let Some(r) = src.version.as_deref() {
                        table.insert("version", toml_edit::value(r));
                    } else {
                        table.remove("version");
                    }
                    table.insert("path", toml_edit::value(relpath));
                    for key in ["git", "branch", "tag", "rev"] {
                        table.remove(key);
                    }
                }
                Some(Source::Git(src)) => {
                    table.insert("git", toml_edit::value(src.git.as_str()));
                    if let Some(branch) = src.branch.as_deref() {
                        table.insert("branch", toml_edit::value(branch));
                    } else {
                        table.remove("branch");
                    }
                    if let Some(tag) = src.tag.as_deref() {
                        table.insert("tag", toml_edit::value(tag));
                    } else {
                        table.remove("tag");
                    }
                    if let Some(rev) = src.rev.as_deref() {
                        table.insert("rev", toml_edit::value(rev));
                    } else {
                        table.remove("rev");
                    }
                    for key in ["version", "path"] {
                        table.remove(key);
                    }
                }
                None => {}
            }
            if table.contains_key("version") {
                if let Some(r) = self.registry.as_deref() {
                    table.insert("registry", toml_edit::value(r));
                } else {
                    table.remove("registry");
                }
            } else {
                table.remove("registry");
            }

            if self.rename.is_some() {
                table.insert("package", toml_edit::value(self.name.as_str()));
            }
            match self.default_features {
                Some(v) => {
                    table.insert("default-features", toml_edit::value(v));
                }
                None => {
                    table.remove("default-features");
                }
            }
            if let Some(new_features) = self.features.as_ref() {
                let mut features = table
                    .get("features")
                    .and_then(|i| i.as_value())
                    .and_then(|v| v.as_array())
                    .and_then(|a| {
                        a.iter()
                            .map(|v| v.as_str())
                            .collect::<Option<IndexSet<_>>>()
                    })
                    .unwrap_or_default();
                features.extend(new_features.iter().map(|s| s.as_str()));
                let features = toml_edit::value(features.into_iter().collect::<toml_edit::Value>());
                table.insert("features", features);
            } else {
                table.remove("features");
            }
            match self.optional {
                Some(v) => {
                    table.insert("optional", toml_edit::value(v));
                }
                None => {
                    table.remove("optional");
                }
            }

            table.fmt();
        } else {
            unreachable!("Invalid dependency type: {}", item.type_name());
        }
    }
}

fn invalid_type(dep: &str, key: &str, actual: &str, expected: &str) -> anyhow::Error {
    anyhow::format_err!("Found {actual} for {key} when {expected} was expected for {dep}")
}

impl std::fmt::Display for Dependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(source) = self.source() {
            write!(f, "{}@{}", self.name, source)
        } else {
            self.toml_key().fmt(f)
        }
    }
}

impl<'s> From<&'s cargo::core::Summary> for Dependency {
    fn from(other: &'s cargo::core::Summary) -> Self {
        Dependency::new(other.name().as_str())
            .set_source(RegistrySource::new(other.version().to_string()))
            .set_available_features_from_cargo(other.features())
    }
}

impl From<cargo::core::Summary> for Dependency {
    fn from(other: cargo::core::Summary) -> Self {
        (&other).into()
    }
}

fn path_field(crate_root: &Path, abs_path: &Path) -> String {
    let relpath = pathdiff::diff_paths(abs_path, crate_root).expect("both paths are absolute");
    let relpath = relpath.to_str().unwrap().replace('\\', "/");
    relpath
}

fn is_package_eq(item: &mut toml_edit::Item, name: &str, rename: Option<&str>) -> bool {
    if let Some(table) = item.as_table_like_mut() {
        let existing_package = table.get("package").and_then(|i| i.as_str());
        let new_package = rename.map(|_| name);
        existing_package == new_package
    } else {
        false
    }
}

/// Primary location of a dependency
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub enum Source {
    /// Dependency from a registry
    Registry(RegistrySource),
    /// Dependency from a local path
    Path(PathSource),
    /// Dependency from a git repo
    Git(GitSource),
}

impl Source {
    /// Access the registry source, if present
    pub fn as_registry(&self) -> Option<&RegistrySource> {
        match self {
            Self::Registry(src) => Some(src),
            _ => None,
        }
    }

    /// Access the path source, if present
    #[allow(dead_code)]
    pub fn as_path(&self) -> Option<&PathSource> {
        match self {
            Self::Path(src) => Some(src),
            _ => None,
        }
    }

    /// Access the git source, if present
    #[allow(dead_code)]
    pub fn as_git(&self) -> Option<&GitSource> {
        match self {
            Self::Git(src) => Some(src),
            _ => None,
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registry(src) => src.fmt(f),
            Self::Path(src) => src.fmt(f),
            Self::Git(src) => src.fmt(f),
        }
    }
}

impl<'s> From<&'s Source> for Source {
    fn from(inner: &'s Source) -> Self {
        inner.clone()
    }
}

impl From<RegistrySource> for Source {
    fn from(inner: RegistrySource) -> Self {
        Self::Registry(inner)
    }
}

impl From<PathSource> for Source {
    fn from(inner: PathSource) -> Self {
        Self::Path(inner)
    }
}

impl From<GitSource> for Source {
    fn from(inner: GitSource) -> Self {
        Self::Git(inner)
    }
}

/// Dependency from a registry
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub struct RegistrySource {
    /// Version requirement
    pub version: String,
}

impl RegistrySource {
    /// Specify dependency by version requirement
    pub fn new(version: impl AsRef<str>) -> Self {
        // versions might have semver metadata appended which we do not want to
        // store in the cargo toml files.  This would cause a warning upon compilation
        // ("version requirement […] includes semver metadata which will be ignored")
        let version = version.as_ref().split('+').next().unwrap();
        Self {
            version: version.to_owned(),
        }
    }
}

impl std::fmt::Display for RegistrySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.version.fmt(f)
    }
}

/// Dependency from a local path
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub struct PathSource {
    /// Local, absolute path
    pub path: PathBuf,
    /// Version requirement for when published
    pub version: Option<String>,
}

impl PathSource {
    /// Specify dependency from a path
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            version: None,
        }
    }

    /// Set an optional version requirement
    pub fn set_version(mut self, version: impl AsRef<str>) -> Self {
        // versions might have semver metadata appended which we do not want to
        // store in the cargo toml files.  This would cause a warning upon compilation
        // ("version requirement […] includes semver metadata which will be ignored")
        let version = version.as_ref().split('+').next().unwrap();
        self.version = Some(version.to_owned());
        self
    }
}

impl std::fmt::Display for PathSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.path.display().fmt(f)
    }
}

/// Dependency from a git repo
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub struct GitSource {
    /// Repo URL
    pub git: String,
    /// Select specific branch
    pub branch: Option<String>,
    /// Select specific tag
    pub tag: Option<String>,
    /// Select specific rev
    pub rev: Option<String>,
}

impl GitSource {
    /// Specify dependency from a git repo
    pub fn new(git: impl Into<String>) -> Self {
        Self {
            git: git.into(),
            branch: None,
            tag: None,
            rev: None,
        }
    }

    /// Specify an optional branch
    pub fn set_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self.tag = None;
        self.rev = None;
        self
    }

    /// Specify an optional tag
    pub fn set_tag(mut self, tag: impl Into<String>) -> Self {
        self.branch = None;
        self.tag = Some(tag.into());
        self.rev = None;
        self
    }

    /// Specify an optional rev
    pub fn set_rev(mut self, rev: impl Into<String>) -> Self {
        self.branch = None;
        self.tag = None;
        self.rev = Some(rev.into());
        self
    }

    fn git_ref(&self) -> cargo::core::GitReference {
        match (
            self.branch.as_deref(),
            self.tag.as_deref(),
            self.rev.as_deref(),
        ) {
            (Some(branch), _, _) => cargo::core::GitReference::Branch(branch.to_owned()),
            (_, Some(tag), _) => cargo::core::GitReference::Tag(tag.to_owned()),
            (_, _, Some(rev)) => cargo::core::GitReference::Rev(rev.to_owned()),
            _ => cargo::core::GitReference::DefaultBranch,
        }
    }
}

impl std::fmt::Display for GitSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let git_ref = self.git_ref();
        if let Some(pretty_ref) = git_ref.pretty_ref() {
            write!(f, "{}?{}", self.git, pretty_ref)
        } else {
            write!(f, "{}", self.git)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn to_toml_simple_dep() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let dep = Dependency::new("dep").set_source(RegistrySource::new("1.0"));
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "dep".to_owned());

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn to_toml_simple_dep_with_version() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let dep = Dependency::new("dep").set_source(RegistrySource::new("1.0"));
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "dep".to_owned());
        assert_eq!(item.as_str(), Some("1.0"));

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn to_toml_optional_dep() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let dep = Dependency::new("dep")
            .set_source(RegistrySource::new("1.0"))
            .set_optional(true);
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "dep".to_owned());
        assert!(item.is_inline_table());

        let dep = item.as_inline_table().unwrap();
        assert_eq!(dep.get("optional").unwrap().as_bool(), Some(true));

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn to_toml_dep_without_default_features() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let dep = Dependency::new("dep")
            .set_source(RegistrySource::new("1.0"))
            .set_default_features(false);
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "dep".to_owned());
        assert!(item.is_inline_table());

        let dep = item.as_inline_table().unwrap();
        assert_eq!(dep.get("default-features").unwrap().as_bool(), Some(false));

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn to_toml_dep_with_path_source() {
        let root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let crate_root = root.join("foo");
        let dep = Dependency::new("dep").set_source(PathSource::new(root.join("bar")));
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "dep".to_owned());
        assert!(item.is_inline_table());

        let dep = item.as_inline_table().unwrap();
        assert_eq!(dep.get("path").unwrap().as_str(), Some("../bar"));

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn to_toml_dep_with_git_source() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let dep = Dependency::new("dep").set_source(GitSource::new("https://foor/bar.git"));
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "dep".to_owned());
        assert!(item.is_inline_table());

        let dep = item.as_inline_table().unwrap();
        assert_eq!(
            dep.get("git").unwrap().as_str(),
            Some("https://foor/bar.git")
        );

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn to_toml_renamed_dep() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let dep = Dependency::new("dep")
            .set_source(RegistrySource::new("1.0"))
            .set_rename("d");
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "d".to_owned());
        assert!(item.is_inline_table());

        let dep = item.as_inline_table().unwrap();
        assert_eq!(dep.get("package").unwrap().as_str(), Some("dep"));

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn to_toml_dep_from_alt_registry() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let dep = Dependency::new("dep")
            .set_source(RegistrySource::new("1.0"))
            .set_registry("alternative");
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "dep".to_owned());
        assert!(item.is_inline_table());

        let dep = item.as_inline_table().unwrap();
        assert_eq!(dep.get("registry").unwrap().as_str(), Some("alternative"));

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn to_toml_complex_dep() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let dep = Dependency::new("dep")
            .set_source(RegistrySource::new("1.0"))
            .set_default_features(false)
            .set_rename("d");
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        assert_eq!(key, "d".to_owned());
        assert!(item.is_inline_table());

        let dep = item.as_inline_table().unwrap();
        assert_eq!(dep.get("package").unwrap().as_str(), Some("dep"));
        assert_eq!(dep.get("version").unwrap().as_str(), Some("1.0"));
        assert_eq!(dep.get("default-features").unwrap().as_bool(), Some(false));

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    fn paths_with_forward_slashes_are_left_as_is() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let path = crate_root.join("sibling/crate");
        let relpath = "sibling/crate";
        let dep = Dependency::new("dep").set_source(PathSource::new(path));
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        let table = item.as_inline_table().unwrap();
        let got = table.get("path").unwrap().as_str().unwrap();
        assert_eq!(got, relpath);

        verify_roundtrip(&crate_root, key, &item);
    }

    #[test]
    #[cfg(windows)]
    fn normalise_windows_style_paths() {
        let crate_root = dunce::canonicalize(Path::new("/")).expect("root exists");
        let original = crate_root.join(r"sibling\crate");
        let should_be = "sibling/crate";
        let dep = Dependency::new("dep").set_source(PathSource::new(original));
        let key = dep.toml_key();
        let item = dep.to_toml(&crate_root);

        let table = item.as_inline_table().unwrap();
        let got = table.get("path").unwrap().as_str().unwrap();
        assert_eq!(got, should_be);

        verify_roundtrip(&crate_root, key, &item);
    }

    #[track_caller]
    fn verify_roundtrip(crate_root: &Path, key: &str, item: &toml_edit::Item) {
        let roundtrip = Dependency::from_toml(crate_root, key, item).unwrap();
        let round_key = roundtrip.toml_key();
        let round_item = roundtrip.to_toml(crate_root);
        assert_eq!(key, round_key);
        assert_eq!(item.to_string(), round_item.to_string());
    }
}
