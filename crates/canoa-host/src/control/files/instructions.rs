//! `fs/instructions`: the instruction files the agents of this host load for a session in a folder. Which files, and where an agent
//! keeps the person's own, is what each agent's description says (`VendorInfo::context_files`, `home_context_files`); nothing here
//! names an agent or a file.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Files, Outcome, links, plain, shown_name, text_of, visible_name};
use crate::control::{Caller, bad, text_param};
use crate::vendors::Vendor;

impl Files {
    /// `fs/instructions` `{"path"}`: for the folder `path`, each existing instruction file of each agent, from the folder up to its
    /// repository's top (or the folder alone outside one), then the agent's own files of the person's. A key that does not hold
    /// `files` is told of the files in the places it browses only.
    pub(crate) fn instructions(&self, caller: &Caller, params: &Value) -> Outcome {
        self.instructions_of(caller, params, &crate::vendors::all(), &|name| {
            std::env::var_os(name)
        })
    }

    fn instructions_of(
        &self,
        caller: &Caller,
        params: &Value,
        vendors: &[Vendor],
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Outcome {
        let asked = text_param(params, "path")?
            .ok_or_else(|| bad("a path is needed: the session's folder"))?;
        let workspace = self.place(caller, asked)?;
        if !fs::metadata(&workspace).is_ok_and(|meta| meta.is_dir()) {
            return Err(bad("that is a file, not a folder"));
        }
        let repository = links::repository_root(&workspace)
            .and_then(|root| fs::canonicalize(root).ok().map(plain))
            .filter(|root| workspace.starts_with(root));
        let stop = repository.as_deref().unwrap_or(&workspace);

        // Each file once, with every agent that loads it.
        let mut found = Vec::<(PathBuf, Value, Vec<&str>)>::new();
        let mut consider = |candidate: PathBuf, vendor: &Vendor, scope: &str, shown: String| {
            let Ok(real) = fs::canonicalize(&candidate).map(plain) else {
                return;
            };
            if !self.within(caller, &real) {
                return;
            }
            let Ok(meta) = fs::metadata(&real) else {
                return;
            };
            if !meta.is_file() {
                return;
            }
            if let Some((_, _, agents)) = found.iter_mut().find(|(path, _, _)| *path == real) {
                if !agents.contains(&vendor.title) {
                    agents.push(vendor.title);
                }
                return;
            }
            let entry = json!({
                "path": text_of(&real),
                "name": visible_name(&shown),
                "scope": scope,
                "size": meta.len(),
            });
            found.push((real, entry, vec![vendor.title]));
        };

        for dir in workspace.ancestors() {
            let scope = if dir == workspace {
                "Workspace"
            } else if repository.as_deref() == Some(dir) {
                "Repository"
            } else {
                "Parent folder"
            };
            for vendor in vendors {
                for file in &vendor.context_files {
                    consider(below(dir, file), vendor, scope, (*file).to_owned());
                }
            }
            if dir == stop {
                break;
            }
        }
        for vendor in vendors {
            let Some(folder) = self.agent_folder(vendor, env) else {
                continue;
            };
            for file in &vendor.home_context_files {
                let shown = shown_in_home(self.home.as_deref(), &folder, file);
                consider(below(&folder, file), vendor, "User instructions", shown);
            }
        }
        let files: Vec<Value> = found
            .into_iter()
            .map(|(_, mut entry, agents)| {
                entry["agents"] = json!(agents);
                entry
            })
            .collect();
        Ok(json!({ "files": files }))
    }

    /// The folder `vendor` keeps its own files in: where its variable names one, else its folder below the person's home.
    fn agent_folder(
        &self,
        vendor: &Vendor,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Option<PathBuf> {
        let named = vendor
            .home_variable
            .and_then(env)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        named.or_else(|| {
            let folder = vendor.home_folder?;
            Some(below(self.home.as_deref()?, folder))
        })
    }
}

/// `file`, written with `/`, below `dir`.
fn below(dir: &Path, file: &str) -> PathBuf {
    file.split('/')
        .filter(|part| !part.is_empty())
        .fold(dir.to_path_buf(), |path, part| path.join(part))
}

/// How a file of an agent's folder is shown: below the person's home as `~/...`, else as its path.
fn shown_in_home(home: Option<&Path>, folder: &Path, file: &str) -> String {
    let path = below(folder, file);
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
        None => shown_name(&path),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::control::{Grant, UploadLimits};

    fn vendor(title: &'static str) -> Vendor {
        Vendor {
            agent: title,
            title,
            vendor: None,
            aos: None,
            model_variable: None,
            tool_name: None,
            skills_folder: None,
            context_files: Vec::new(),
            home_variable: None,
            home_folder: None,
            home_context_files: Vec::new(),
            standards: Vec::new(),
            names: Vec::new(),
        }
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canoa-instr-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("directory");
        fs::canonicalize(&dir).map(plain).expect("resolved")
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("a parent")).expect("folder");
        fs::write(path, text).expect("file");
    }

    fn owner_files(home: &Path) -> Files {
        Files::new(
            Some(home.to_path_buf()),
            Vec::new(),
            None,
            None,
            UploadLimits::default(),
        )
    }

    fn names(answer: &Value) -> Vec<(String, Vec<String>)> {
        answer["files"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|each| {
                (
                    each["name"].as_str().expect("a name").to_owned(),
                    each["agents"]
                        .as_array()
                        .expect("agents")
                        .iter()
                        .map(|agent| agent.as_str().expect("text").to_owned())
                        .collect(),
                )
            })
            .collect()
    }

    fn two() -> Vec<Vendor> {
        let mut alpha = vendor("Alpha");
        alpha.context_files = vec!["ALPHA.md", ".alpha/ALPHA.md"];
        alpha.home_variable = Some("ALPHA_HOME");
        alpha.home_folder = Some(".alpha");
        alpha.home_context_files = vec!["ALPHA.md"];
        let mut beta = vendor("Beta");
        beta.context_files = vec!["SHARED.md"];
        let mut gamma = vendor("Gamma");
        gamma.context_files = vec!["SHARED.md"];
        vec![alpha, beta, gamma]
    }

    #[test]
    fn it_lists_the_files_each_description_names_from_the_folder_up_to_the_repository_and_no_others()
     {
        let tmp = dir("walk");
        let (home, repo) = (tmp.join("home"), tmp.join("repo"));
        let folder = repo.join("pkg").join("inner");
        fs::create_dir_all(repo.join(".git")).expect("repository");
        fs::create_dir_all(&folder).expect("folder");
        put(&folder.join("ALPHA.md"), "a");
        put(&repo.join(".alpha").join("ALPHA.md"), "b");
        put(&repo.join("pkg").join("SHARED.md"), "c");
        put(&repo.join("CLAUDE.md"), "named by no description here");
        // Above the repository's top nothing is looked at.
        put(&tmp.join("ALPHA.md"), "outside");
        let files = owner_files(&home);
        let answer = files
            .instructions_of(
                &Caller::Owner,
                &json!({"path": text_of(&folder)}),
                &two(),
                &|_| None,
            )
            .expect("answered");
        assert_eq!(
            names(&answer),
            vec![
                ("ALPHA.md".to_owned(), vec!["Alpha".to_owned()]),
                (
                    "SHARED.md".to_owned(),
                    vec!["Beta".to_owned(), "Gamma".to_owned()]
                ),
                (".alpha/ALPHA.md".to_owned(), vec!["Alpha".to_owned()]),
            ],
            "{answer}"
        );
        let scopes: Vec<&str> = answer["files"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|each| each["scope"].as_str().expect("scope"))
            .collect();
        assert_eq!(scopes, ["Workspace", "Parent folder", "Repository"]);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn the_agents_own_folder_is_the_one_its_variable_names_else_its_folder_in_the_home() {
        let tmp = dir("home");
        let (home, work, moved) = (tmp.join("home"), tmp.join("work"), tmp.join("moved"));
        fs::create_dir_all(&work).expect("folder");
        put(&home.join(".alpha").join("ALPHA.md"), "default");
        put(&moved.join("ALPHA.md"), "moved");
        let files = owner_files(&home);
        let ask = |env: &dyn Fn(&str) -> Option<OsString>| {
            files
                .instructions_of(
                    &Caller::Owner,
                    &json!({"path": text_of(&work)}),
                    &two(),
                    env,
                )
                .expect("answered")
        };
        let unset = ask(&|_| None);
        assert_eq!(
            names(&unset),
            vec![("~/.alpha/ALPHA.md".to_owned(), vec!["Alpha".to_owned()])]
        );
        assert_eq!(unset["files"][0]["scope"], "User instructions");
        let set: BTreeMap<&str, OsString> = [("ALPHA_HOME", moved.clone().into_os_string())].into();
        let named = ask(&|name| set.get(name).cloned());
        assert_eq!(names(&named).len(), 1, "{named}");
        assert_eq!(
            fs::canonicalize(named["files"][0]["path"].as_str().expect("path"))
                .map(plain)
                .expect("resolved"),
            moved.join("ALPHA.md")
        );
        // A variable that is not an absolute path is not a folder to look in: the default applies.
        let relative: BTreeMap<&str, OsString> = [("ALPHA_HOME", OsString::from("moved"))].into();
        let fallen = ask(&|name| relative.get(name).cloned());
        assert_eq!(names(&fallen), names(&unset));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_key_without_files_is_told_only_of_what_is_in_the_places_it_browses() {
        let tmp = dir("keyed");
        let (home, outside) = (tmp.join("home"), tmp.join("outside"));
        fs::create_dir_all(&home).expect("home");
        put(&home.join(".alpha").join("ALPHA.md"), "own");
        put(&outside.join("ALPHA.md"), "elsewhere");
        let files = owner_files(&home);
        let key = |grants: &[Grant]| Caller::Peer {
            id: "sha256:aa".to_owned(),
            grants: grants.iter().copied().collect(),
        };
        let browsing = key(&[Grant::Folders]);
        let refused = files.instructions_of(
            &browsing,
            &json!({"path": text_of(&outside)}),
            &two(),
            &|_| None,
        );
        assert!(
            refused.is_err(),
            "a place outside the browsable ones is not read"
        );
        let inside = files
            .instructions_of(&browsing, &json!({"path": text_of(&home)}), &two(), &|_| {
                None
            })
            .expect("answered");
        assert_eq!(names(&inside).len(), 1, "{inside}");
        let whole = key(&[Grant::Folders, Grant::Files]);
        let free = files
            .instructions_of(&whole, &json!({"path": text_of(&outside)}), &two(), &|_| {
                None
            })
            .expect("answered");
        assert_eq!(names(&free).len(), 2, "{free}");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn the_bundled_agents_describe_their_own_files() {
        let all = crate::vendors::all();
        let find = |agent: &str| {
            all.iter()
                .find(|each| each.agent == agent)
                .expect("bundled")
        };
        let claude = find("claude");
        assert_eq!(claude.home_variable, Some("CLAUDE_CONFIG_DIR"));
        assert!(claude.context_files.contains(&"CLAUDE.md"));
        assert_eq!(claude.home_context_files, ["CLAUDE.md"]);
        let codex = find("codex");
        assert_eq!(codex.home_variable, Some("CODEX_HOME"));
        assert!(codex.context_files.contains(&"AGENTS.md"));
    }
}
