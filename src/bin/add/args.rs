///! Handle `cargo add` arguments

use semver;
use std::error::Error;
use cargo_edit::Dependency;
use fetch_version::get_latest_version;

macro_rules! toml_table {
    ($($key:expr => $value:expr),+) => {
        {
            let mut dep = BTreeMap::new();
            $(dep.insert(String::from($key), $value);)+
            toml::Value::Table(dep)
        }
    }
}

#[derive(Debug, RustcDecodable)]
/// Docopts input args.
pub struct Args {
    /// Crate name
    pub arg_crate: String,
    /// dev-dependency
    pub flag_dev: bool,
    /// build-dependency
    pub flag_build: bool,
    /// Version
    pub flag_vers: Option<String>,
    /// Git repo Path
    pub flag_git: Option<String>,
    /// Crate directory path
    pub flag_path: Option<String>,
    /// Optional dependency
    pub flag_optional: bool,
    /// `Cargo.toml` path
    pub flag_manifest_path: Option<String>,
    /// `--version`
    pub flag_version: bool,
}

impl Args {
    /// Get depenency section
    pub fn get_section(&self) -> &'static str {
        if self.flag_dev {
            "dev-dependencies"
        } else if self.flag_build {
            "build-dependencies"
        } else {
            "dependencies"
        }
    }

    /// Build depenency from arguments
    pub fn parse_dependency(&self) -> Result<Dependency, Box<Error>> {
        if crate_name_has_version(&self.arg_crate) {
            return parse_crate_name_with_version(&self.arg_crate);
        } else if crate_name_has_tilde(&self.arg_crate) {
			return parse_crate_name_with_tilde(&self.arg_crate); 
		}

        let dependency = Dependency::new(&self.arg_crate).set_optional(self.flag_optional);

        let dependency = if let Some(ref version) = self.flag_vers {
            try!(semver::VersionReq::parse(&version));
            dependency.set_version(version)
        } else if let Some(ref repo) = self.flag_git {
            dependency.set_git(repo)
        } else if let Some(ref path) = self.flag_path {
            dependency.set_path(path)
        } else {
            let v = try!(get_latest_version(&self.arg_crate));
            dependency.set_version(&v)
        };

        Ok(dependency)
    }
}

impl Default for Args {
    fn default() -> Args {
        Args {
            arg_crate: "demo".to_owned(),
            flag_dev: false,
            flag_build: false,
            flag_vers: None,
            flag_git: None,
            flag_path: None,
            flag_optional: false,
            flag_manifest_path: None,
            flag_version: false,
        }
    }
}

fn crate_name_has_version(name: &str) -> bool {
    name.contains("@")
}

fn crate_name_has_tilde(name: &str) -> bool {
    name.ends_with("~")
}

fn parse_crate_name_with_version(name: &str) -> Result<Dependency, Box<Error>> {
    let xs: Vec<&str> = name.splitn(2, "@").collect();
    let mut version = "^".to_owned();
    let (name, version_num) = (xs[0], xs[1]);
    version.push_str(&*version_num);
    if version_num.len() > 0 {
		match version_num.chars().nth(0).unwrap() {
			'0'...'9' => { }, // for simple numeric versions add caret prefix 
			_ => version = version_num.into(), // this is not a simple numeric version, keep existing prefix
		}
	} // else, it will fail below
	
    try!(semver::VersionReq::parse(&version));

    Ok(Dependency::new(name).set_version(&*version))
}

fn parse_crate_name_with_tilde(name: &str) -> Result<Dependency, Box<Error>> {
	let name_without_tilde = &name[..(name.len() - 1)];
	let mut v = "~".to_owned();
	v.push_str(&*try!(get_latest_version(name_without_tilde)));
    Ok(Dependency::new(name_without_tilde).set_version(&*v))
}

#[cfg(test)]
mod tests {
    use cargo_edit::Dependency;
    use super::*;
    use fetch_version::get_latest_version; 
    
    #[test]
    fn test_dependency_parsing() {
        let args = Args {
            arg_crate: "demo".to_owned(),
            flag_vers: Some("0.4.2".to_owned()),
            ..Args::default()
        };

        assert_eq!(args.parse_dependency().unwrap(),
                   Dependency::new("demo").set_version("0.4.2"));
    }
    
    #[test]
    fn test_tilde_dependency_parsing() {
        let args = Args {
            arg_crate: "rand~".to_owned(),
            flag_vers: None,
            ..Args::default()
        };
        let mut v = "~".to_owned();
		v.push_str(&*get_latest_version("rand").unwrap());
        
        assert_eq!(args.parse_dependency().unwrap(),
			Dependency::new("rand").set_version(&*v));
    }
}
