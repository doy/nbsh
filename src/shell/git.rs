#[derive(Debug)]
pub struct Info {
    modified_files: bool,
    staged_files: bool,
    new_files: bool,
    commits: bool,
    active_operation: ActiveOperation,
    branch: Option<String>,
    remote_branch_diff: Option<(usize, usize)>,
}

const MODIFIED: git2::Status = git2::Status::WT_DELETED
    .union(git2::Status::WT_MODIFIED)
    .union(git2::Status::WT_RENAMED)
    .union(git2::Status::WT_TYPECHANGE)
    .union(git2::Status::CONFLICTED);
const STAGED: git2::Status = git2::Status::INDEX_DELETED
    .union(git2::Status::INDEX_MODIFIED)
    .union(git2::Status::INDEX_NEW)
    .union(git2::Status::INDEX_RENAMED)
    .union(git2::Status::INDEX_TYPECHANGE);
const NEW: git2::Status = git2::Status::WT_NEW;

impl Info {
    pub fn new(git: &git2::Repository) -> Self {
        let mut status_options = git2::StatusOptions::new();
        status_options.include_untracked(true);
        status_options.update_index(true);

        let statuses = git.statuses(Some(&mut status_options));

        let mut modified_files = false;
        let mut staged_files = false;
        let mut new_files = false;
        if let Ok(statuses) = statuses {
            for file in statuses.iter() {
                if file.status().intersects(MODIFIED) {
                    modified_files = true;
                }
                if file.status().intersects(STAGED) {
                    staged_files = true;
                }
                if file.status().intersects(NEW) {
                    new_files = true;
                }
            }
        }

        let head = git.head();
        let mut commits = false;
        let mut branch = None;
        let mut remote_branch_diff = None;

        if let Ok(head) = head {
            commits = true;
            if head.is_branch() {
                branch = head.shorthand().map(ToString::to_string);
                remote_branch_diff =
                    head.resolve()
                        .ok()
                        .map(|head| {
                            (
                                head.target(),
                                head.shorthand().map(ToString::to_string),
                            )
                        })
                        .and_then(|(head_id, name)| {
                            head_id.and_then(|head_id| {
                                name.and_then(|name| {
                                    git.refname_to_id(&format!(
                                        "refs/remotes/origin/{}",
                                        name
                                    ))
                                    .ok()
                                    .and_then(|remote_id| {
                                        git.graph_ahead_behind(
                                            head_id, remote_id,
                                        )
                                        .ok()
                                    })
                                })
                            })
                        });
            } else {
                branch =
                    head.resolve().ok().and_then(|head| head.target()).map(
                        |oid| {
                            let mut sha: String = oid
                                .as_bytes()
                                .iter()
                                .take(4)
                                .map(|b| format!("{:02x}", b))
                                .collect();
                            sha.truncate(7);
                            sha
                        },
                    );
            }
        }

        let active_operation = match git.state() {
            git2::RepositoryState::Merge => ActiveOperation::Merge,
            git2::RepositoryState::Revert
            | git2::RepositoryState::RevertSequence => {
                ActiveOperation::Revert
            }
            git2::RepositoryState::CherryPick
            | git2::RepositoryState::CherryPickSequence => {
                ActiveOperation::CherryPick
            }
            git2::RepositoryState::Bisect => ActiveOperation::Bisect,
            git2::RepositoryState::Rebase
            | git2::RepositoryState::RebaseInteractive
            | git2::RepositoryState::RebaseMerge => ActiveOperation::Rebase,
            _ => ActiveOperation::None,
        };

        Self {
            modified_files,
            staged_files,
            new_files,
            commits,
            active_operation,
            branch,
            remote_branch_diff,
        }
    }
}

impl std::fmt::Display for Info {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "g")?;

        if self.modified_files {
            write!(f, "*")?;
        }
        if self.staged_files {
            write!(f, "+")?;
        }
        if self.new_files {
            write!(f, "?")?;
        }
        if !self.commits {
            write!(f, "!")?;
            return Ok(());
        }

        let branch = self.branch.as_ref().map_or("???", |branch| {
            if branch == "master" {
                ""
            } else {
                branch
            }
        });
        if !branch.is_empty() {
            write!(f, ":")?;
        }
        write!(f, "{}", branch)?;

        if let Some((local, remote)) = self.remote_branch_diff {
            if local > 0 || remote > 0 {
                write!(f, ":")?;
            }
            if local > 0 {
                write!(f, "+{}", local)?;
            }
            if remote > 0 {
                write!(f, "-{}", remote)?;
            }
        } else {
            write!(f, ":-")?;
        }

        write!(f, "{}", self.active_operation)?;

        Ok(())
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ActiveOperation {
    None,
    Merge,
    Revert,
    CherryPick,
    Bisect,
    Rebase,
}

impl std::fmt::Display for ActiveOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActiveOperation::None => Ok(()),
            ActiveOperation::Merge => write!(f, "(m)"),
            ActiveOperation::Revert => write!(f, "(v)"),
            ActiveOperation::CherryPick => write!(f, "(c)"),
            ActiveOperation::Bisect => write!(f, "(b)"),
            ActiveOperation::Rebase => write!(f, "(r)"),
        }
    }
}
