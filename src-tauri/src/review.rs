use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: String, // "context" | "add" | "del"
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub file_path: String,
    pub abs_path: String,
    pub status: DiffStatus,
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    pub provider: String, // "github" | "gitlab"
    pub number: u64,
    pub title: String,
    pub url: String,
    pub base_branch: String,
    pub head_branch: String,
    pub head_sha: String,
    #[serde(default)]
    pub repo_name: String,
    #[serde(default)]
    pub checkout_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffSummary {
    pub branch: String,
    pub base_ref: String,
    pub files: Vec<FileDiff>,
    pub pr: Option<PrInfo>,
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize, usize, usize)> {
    // @@ -old_start,old_lines +new_start,new_lines @@
    let rest = line.strip_prefix("@@ ")?;
    let end = rest.find(" @@")?;
    let range_part = &rest[..end];
    let mut parts = range_part.split_whitespace();
    let old_part = parts.next()?.strip_prefix('-')?;
    let new_part = parts.next()?.strip_prefix('+')?;

    let parse_pair = |s: &str| -> Option<(usize, usize)> {
        if let Some((a, b)) = s.split_once(',') {
            Some((a.parse().ok()?, b.parse().ok()?))
        } else {
            Some((s.parse().ok()?, 1))
        }
    };
    let (os, ol) = parse_pair(old_part)?;
    let (ns, nl) = parse_pair(new_part)?;
    Some((os, ol, ns, nl))
}

const MAX_DIFF_LINES_PER_FILE: usize = 1500;

pub fn parse_hunks_from_patch(patch: &str) -> (Vec<DiffHunk>, usize, usize) {
    let mut hunks = Vec::new();
    let mut current: Option<DiffHunk> = None;
    let mut old_ln = 0usize;
    let mut new_ln = 0usize;
    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut stored_lines = 0usize;
    let mut truncated = false;

    for raw in patch.lines() {
        if raw.starts_with("@@ ") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            if stored_lines >= MAX_DIFF_LINES_PER_FILE {
                truncated = true;
                continue;
            }
            if let Some((os, ol, ns, nl)) = parse_hunk_header(raw) {
                old_ln = os;
                new_ln = ns;
                current = Some(DiffHunk {
                    old_start: os,
                    old_lines: ol,
                    new_start: ns,
                    new_lines: nl,
                    lines: Vec::new(),
                });
            }
            continue;
        }
        if let Some(text) = raw.strip_prefix('+') {
            additions += 1;
            if let Some(h) = current.as_mut() {
                if stored_lines < MAX_DIFF_LINES_PER_FILE {
                    h.lines.push(DiffLine {
                        kind: "add".to_string(),
                        old_line: None,
                        new_line: Some(new_ln),
                        text: text.to_string(),
                    });
                    stored_lines += 1;
                } else {
                    truncated = true;
                }
            }
            new_ln += 1;
        } else if let Some(text) = raw.strip_prefix('-') {
            deletions += 1;
            if let Some(h) = current.as_mut() {
                if stored_lines < MAX_DIFF_LINES_PER_FILE {
                    h.lines.push(DiffLine {
                        kind: "del".to_string(),
                        old_line: Some(old_ln),
                        new_line: None,
                        text: text.to_string(),
                    });
                    stored_lines += 1;
                } else {
                    truncated = true;
                }
            }
            old_ln += 1;
        } else if let Some(text) = raw.strip_prefix(' ') {
            if let Some(h) = current.as_mut() {
                if stored_lines < MAX_DIFF_LINES_PER_FILE {
                    h.lines.push(DiffLine {
                        kind: "context".to_string(),
                        old_line: Some(old_ln),
                        new_line: Some(new_ln),
                        text: text.to_string(),
                    });
                    stored_lines += 1;
                } else {
                    truncated = true;
                }
            }
            old_ln += 1;
            new_ln += 1;
        }
    }
    if let Some(mut h) = current {
        if truncated {
            h.lines.push(DiffLine {
                kind: "context".to_string(),
                old_line: None,
                new_line: None,
                text: format!("... diff truncated (showing first {MAX_DIFF_LINES_PER_FILE} lines) ..."),
            });
        }
        hunks.push(h);
    } else if truncated {
        if let Some(last) = hunks.last_mut() {
            last.lines.push(DiffLine {
                kind: "context".to_string(),
                old_line: None,
                new_line: None,
                text: format!("... diff truncated (showing first {MAX_DIFF_LINES_PER_FILE} lines) ..."),
            });
        }
    }
    (hunks, additions, deletions)
}

pub fn parse_unified_diff(root: &Path, diff_text: &str) -> Vec<FileDiff> {
    let mut files = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_status = DiffStatus::Modified;
    let mut patch_buf = String::new();

    let flush = |files: &mut Vec<FileDiff>,
                 path_opt: Option<String>,
                 status: DiffStatus,
                 buf: &str| {
        let Some(rel) = path_opt else {
            return;
        };
        let (hunks, additions, deletions) = parse_hunks_from_patch(buf);
        let abs_path = root.join(&rel).to_string_lossy().into_owned();
        files.push(FileDiff {
            file_path: rel,
            abs_path,
            status,
            additions,
            deletions,
            hunks,
        });
    };

    for line in diff_text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush(&mut files, current_path.take(), current_status.clone(), &patch_buf);
            patch_buf.clear();
            current_status = DiffStatus::Modified;
            if let Some((_, b_part)) = rest.split_once(" b/") {
                current_path = Some(b_part.to_string());
            }
            continue;
        }
        if line.starts_with("new file mode") {
            current_status = DiffStatus::Added;
        } else if line.starts_with("deleted file mode") {
            current_status = DiffStatus::Deleted;
        } else if line.starts_with("@@ ")
            || line.starts_with('+') && !line.starts_with("+++ ")
            || line.starts_with('-') && !line.starts_with("--- ")
            || line.starts_with(' ')
        {
            patch_buf.push_str(line);
            patch_buf.push('\n');
        }
    }
    flush(&mut files, current_path, current_status, &patch_buf);
    files
}

fn run_git(project_path: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(project_path)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "git command failed".to_string()
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn get_git_diff(project_path: &str, base_ref: Option<String>) -> Result<GitDiffSummary, String> {
    let root = Path::new(project_path);
    if !root.is_dir() {
        return Err(format!("Project directory does not exist: {project_path}"));
    }
    let branch = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "HEAD".to_string());

    let base = base_ref
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HEAD".to_string());

    let diff_out = run_git(root, &["diff", "--unified=100", "--no-color", &base])?;
    let mut files = parse_unified_diff(root, &diff_out);

    // Include untracked files when diffing against working tree HEAD
    if base == "HEAD" {
        if let Ok(status_out) = run_git(root, &["status", "--porcelain"]) {
            for line in status_out.lines() {
                if let Some(rel) = line.strip_prefix("?? ") {
                    let rel = rel.trim();
                    let abs = root.join(rel);
                    if abs.is_file() && !files.iter().any(|f| f.file_path == rel) {
                        let content = fs::read_to_string(&abs).unwrap_or_default();
                        let line_cnt = content.lines().count();
                        let lines: Vec<DiffLine> = content
                            .lines()
                            .enumerate()
                            .take(200)
                            .map(|(i, t)| DiffLine {
                                kind: "add".to_string(),
                                old_line: None,
                                new_line: Some(i + 1),
                                text: t.to_string(),
                            })
                            .collect();
                        files.push(FileDiff {
                            file_path: rel.to_string(),
                            abs_path: abs.to_string_lossy().into_owned(),
                            status: DiffStatus::Added,
                            additions: line_cnt,
                            deletions: 0,
                            hunks: if lines.is_empty() {
                                vec![]
                            } else {
                                vec![DiffHunk {
                                    old_start: 0,
                                    old_lines: 0,
                                    new_start: 1,
                                    new_lines: lines.len(),
                                    lines,
                                }]
                            },
                        });
                    }
                }
            }
        }
    }

    Ok(GitDiffSummary {
        branch,
        base_ref: base,
        files,
        pr: None,
    })
}

#[derive(Clone)]
enum ParsedPrTarget {
    GitHub {
        owner: String,
        repo: String,
        number: u64,
    },
    GitLab {
        host: String,
        project_path: String,
        iid: u64,
    },
}

fn parse_remote_repo(project_path: &Path) -> Option<(String, String, String)> {
    let url = run_git(project_path, &["remote", "get-url", "origin"]).ok()?;
    let url = url.trim();
    // git@github.com:owner/repo.git, ssh://git@github.com/owner/repo.git, or https://github.com/owner/repo.git
    if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        let host = host.split(':').next().unwrap_or(host);
        let clean = path.trim_end_matches(".git").to_string();
        return Some(("ssh".to_string(), host.to_string(), clean));
    }
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        let clean = path.trim_end_matches(".git").to_string();
        return Some(("ssh".to_string(), host.to_string(), clean));
    }
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        let (host, path) = rest.split_once('/')?;
        let clean = path.trim_end_matches(".git").to_string();
        return Some(("https".to_string(), host.to_string(), clean));
    }
    None
}

fn resolve_pr_target(project_path: &Path, input: &str) -> Result<ParsedPrTarget, String> {
    let trimmed = input.trim();
    if trimmed.contains("github.com/") {
        let after = trimmed
            .split("github.com/")
            .nth(1)
            .ok_or("Invalid GitHub PR URL")?;
        let segs: Vec<&str> = after.split('/').collect();
        if segs.len() >= 4 && segs[2] == "pull" {
            let num: u64 = segs[3]
                .split(&['?', '#'][..])
                .next()
                .unwrap_or("")
                .parse()
                .map_err(|_| "Invalid GitHub PR number")?;
            return Ok(ParsedPrTarget::GitHub {
                owner: segs[0].to_string(),
                repo: segs[1].to_string(),
                number: num,
            });
        }
    }
    if trimmed.contains("/-/merge_requests/") {
        let without_proto = trimmed
            .strip_prefix("https://")
            .or_else(|| trimmed.strip_prefix("http://"))
            .unwrap_or(trimmed);
        let (host, rest) = without_proto
            .split_once('/')
            .ok_or("Invalid GitLab MR URL")?;
        let (proj, mr_part) = rest
            .split_once("/-/merge_requests/")
            .ok_or("Invalid GitLab MR URL")?;
        let iid: u64 = mr_part
            .split(&['?', '#', '/'][..])
            .next()
            .unwrap_or("")
            .parse()
            .map_err(|_| "Invalid GitLab MR number")?;
        return Ok(ParsedPrTarget::GitLab {
            host: host.to_string(),
            project_path: proj.to_string(),
            iid,
        });
    }

    // Numeric PR/MR number using git origin remote
    let num: u64 = trimmed
        .trim_start_matches('#')
        .parse()
        .map_err(|_| "Enter a full GitHub/GitLab PR URL or #number".to_string())?;
    let (_, host, repo_path) = parse_remote_repo(project_path)
        .ok_or_else(|| "Could not detect origin remote; pass full PR/MR URL".to_string())?;
    if host.contains("gitlab") {
        Ok(ParsedPrTarget::GitLab {
            host,
            project_path: repo_path,
            iid: num,
        })
    } else {
        let (owner, repo) = repo_path
            .split_once('/')
            .ok_or_else(|| "Invalid GitHub origin remote".to_string())?;
        Ok(ParsedPrTarget::GitHub {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number: num,
        })
    }
}

impl ParsedPrTarget {
    fn repo_name(&self) -> String {
        match self {
            ParsedPrTarget::GitHub { repo, .. } => repo.clone(),
            ParsedPrTarget::GitLab { project_path, .. } => project_path
                .rsplit('/')
                .next()
                .unwrap_or(project_path)
                .to_string(),
        }
    }

    fn checkout_slug(&self) -> String {
        let raw = match self {
            ParsedPrTarget::GitHub { owner, repo, number } => format!("{owner}-{repo}-pr-{number}"),
            ParsedPrTarget::GitLab { project_path, iid, .. } => format!("{project_path}-mr-{iid}"),
        };
        raw.chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect()
    }

    fn pr_ref(&self) -> String {
        match self {
            ParsedPrTarget::GitHub { number, .. } => format!("refs/pull/{number}/head"),
            ParsedPrTarget::GitLab { iid, .. } => format!("refs/merge-requests/{iid}/head"),
        }
    }

    fn https_clone_url(&self) -> String {
        match self {
            ParsedPrTarget::GitHub { owner, repo, .. } => {
                format!("https://github.com/{owner}/{repo}.git")
            }
            ParsedPrTarget::GitLab { host, project_path, .. } => {
                format!("https://{host}/{project_path}.git")
            }
        }
    }

    fn matches_remote(&self, remote_path: &str) -> bool {
        match self {
            ParsedPrTarget::GitHub { owner, repo, .. } => {
                remote_path.eq_ignore_ascii_case(&format!("{owner}/{repo}"))
            }
            ParsedPrTarget::GitLab { project_path, .. } => {
                remote_path.eq_ignore_ascii_case(project_path)
            }
        }
    }
}

fn with_https_token(url: &str, token: Option<&str>, is_gitlab: bool) -> String {
    let Some(tok) = token.map(str::trim).filter(|s| !s.is_empty()) else {
        return url.to_string();
    };
    let Some(rest) = url.strip_prefix("https://") else {
        return url.to_string();
    };
    let user = if is_gitlab { "oauth2" } else { "x-access-token" };
    format!("https://{user}:{tok}@{rest}")
}

fn checkout_pr_to_temp_dir(
    source_repo: &Path,
    target: &ParsedPrTarget,
    head_branch: Option<&str>,
    expected_head_sha: Option<&str>,
    token: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    let checkout_dir = std::env::temp_dir()
        .join("vcode-pr")
        .join(target.checkout_slug());
    fs::create_dir_all(&checkout_dir)
        .map_err(|e| format!("Failed to create PR temp directory: {e}"))?;

    if checkout_dir.join(".git").exists() {
        if let Some(sha) = expected_head_sha.map(str::trim).filter(|s| !s.is_empty()) {
            if let Ok(local_sha) = run_git(&checkout_dir, &["rev-parse", "HEAD"]) {
                if local_sha.trim().eq_ignore_ascii_case(sha) {
                    return Ok(checkout_dir);
                }
            }
        }
    }

    let local_origin = if source_repo.exists() {
        parse_remote_repo(source_repo).and_then(|(_, _, remote_path)| {
            if target.matches_remote(&remote_path) {
                run_git(source_repo, &["remote", "get-url", "origin"])
                    .ok()
                    .map(|u| u.trim().to_string())
            } else {
                None
            }
        })
    } else {
        None
    };

    let clone_url = local_origin.unwrap_or_else(|| target.https_clone_url());
    let is_gitlab = matches!(target, ParsedPrTarget::GitLab { .. });
    let fetch_url = with_https_token(&clone_url, token, is_gitlab);

    if !checkout_dir.join(".git").exists() {
        run_git(&checkout_dir, &["init"])?;
        run_git(&checkout_dir, &["remote", "add", "origin", &clone_url])?;
    } else {
        let _ = run_git(&checkout_dir, &["remote", "set-url", "origin", &clone_url]);
    }

    let pr_ref = target.pr_ref();
    if let Err(err) = run_git(&checkout_dir, &["fetch", "--depth", "1", &fetch_url, &pr_ref]) {
        if let Some(hb) = head_branch.map(str::trim).filter(|s| !s.is_empty()) {
            run_git(&checkout_dir, &["fetch", "--depth", "1", &fetch_url, hb])
                .map_err(|e2| format!("Failed to fetch PR ref ({err}; branch fallback: {e2})"))?;
        } else {
            return Err(format!("Failed to fetch PR ref: {err}"));
        }
    }

    run_git(&checkout_dir, &["checkout", "-f", "FETCH_HEAD"])?;
    let _ = run_git(&checkout_dir, &["clean", "-fd"]);
    Ok(checkout_dir)
}

fn find_cli_binary(program: &str) -> std::path::PathBuf {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home_p = std::path::PathBuf::from(home);
        candidates.push(home_p.join(".local/bin").join(program));
        candidates.push(home_p.join("bin").join(program));
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
        "/home/linuxbrew/.linuxbrew/bin",
    ] {
        candidates.push(Path::new(dir).join(program));
    }
    for p in candidates {
        if p.is_file() {
            return p;
        }
    }
    std::path::PathBuf::from(program)
}

fn resolve_cli_token(program: &str, args: &[&str]) -> Option<String> {
    let bin = find_cli_binary(program);
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let tok = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if tok.is_empty() {
        None
    } else {
        Some(tok)
    }
}

fn resolve_github_token(explicit: Option<String>) -> Option<String> {
    if let Some(t) = explicit.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        return Some(t);
    }
    std::env::var("GITHUB_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("GH_TOKEN").ok().filter(|s| !s.trim().is_empty()))
        .or_else(|| resolve_cli_token("gh", &["auth", "token"]))
}

fn resolve_token(target: &ParsedPrTarget, explicit: Option<String>) -> Option<String> {
    if let Some(t) = explicit.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        return Some(t);
    }
    match target {
        ParsedPrTarget::GitHub { .. } => resolve_github_token(None),
        ParsedPrTarget::GitLab { .. } => std::env::var("GITLAB_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| resolve_cli_token("glab", &["config", "get", "token"])),
    }
}

pub fn parse_github_repo_url(input: &str) -> Result<(String, String, String), String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Enter a GitHub repository URL or owner/repo".to_string());
    }
    let path_part = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"))
    {
        rest
    } else if !trimmed.contains("://") && !trimmed.contains('@') {
        trimmed
    } else {
        return Err("Enter a valid GitHub repository URL (e.g. https://github.com/owner/repo)".to_string());
    };

    let mut segs = path_part.split('/').filter(|s| !s.is_empty());
    let owner = segs
        .next()
        .ok_or_else(|| "Missing repository owner".to_string())?;
    let repo_raw = segs
        .next()
        .ok_or_else(|| "Missing repository name".to_string())?;
    let repo = repo_raw.strip_suffix(".git").unwrap_or(repo_raw);
    if owner.is_empty() || repo.is_empty() {
        return Err("Invalid GitHub repository URL".to_string());
    }
    let clone_url = format!("https://github.com/{owner}/{repo}.git");
    Ok((owner.to_string(), repo.to_string(), clone_url))
}

pub async fn clone_github_repo(repo_url: &str, token: Option<String>) -> Result<String, String> {
    let (owner, repo, clone_url) = parse_github_repo_url(repo_url)?;
    let tok = resolve_github_token(token);
    let slug: String = format!("{owner}-{repo}")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();

    let repo_dir = tauri::async_runtime::spawn_blocking(move || -> Result<std::path::PathBuf, String> {
        let dir = std::env::temp_dir().join("vcode-repos").join(&slug);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create temp repo directory: {e}"))?;
        let fetch_url = with_https_token(&clone_url, tok.as_deref(), false);

        if !dir.join(".git").exists() {
            run_git(&dir, &["init"])?;
            run_git(&dir, &["remote", "add", "origin", &clone_url])?;
        } else {
            let _ = run_git(&dir, &["remote", "set-url", "origin", &clone_url]);
        }
        run_git(&dir, &["fetch", "--depth", "1", &fetch_url, "HEAD"])?;
        run_git(&dir, &["checkout", "-f", "FETCH_HEAD"])?;
        let _ = run_git(&dir, &["clean", "-fd"]);
        Ok(dir)
    })
    .await
    .map_err(|e| format!("Clone task failed: {e}"))??;

    Ok(repo_dir.to_string_lossy().into_owned())
}

fn fetch_pr_via_cli(
    root: &Path,
    target: &ParsedPrTarget,
    provider: &str,
    number: u64,
    url: String,
    token: Option<&str>,
) -> Result<GitDiffSummary, String> {
    let out = if provider == "github" {
        Command::new(find_cli_binary("gh"))
            .current_dir(root)
            .args(["pr", "diff", &number.to_string()])
            .output()
            .map_err(|e| format!("Failed to run gh: {e}"))?
    } else {
        Command::new(find_cli_binary("glab"))
            .current_dir(root)
            .args(["mr", "diff", &number.to_string()])
            .output()
            .map_err(|e| format!("Failed to run glab: {e}"))?
    };
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let checkout_dir = checkout_pr_to_temp_dir(root, target, None, None, token)?;
    let diff_out = String::from_utf8_lossy(&out.stdout).into_owned();
    let files = parse_unified_diff(&checkout_dir, &diff_out);
    Ok(GitDiffSummary {
        branch: format!("pr-{number}"),
        base_ref: "base".to_string(),
        files,
        pr: Some(PrInfo {
            provider: provider.to_string(),
            number,
            title: format!("PR #{number}"),
            url,
            base_branch: "base".to_string(),
            head_branch: format!("pr-{number}"),
            head_sha: String::new(),
            repo_name: target.repo_name(),
            checkout_path: Some(checkout_dir.to_string_lossy().into_owned()),
        }),
    })
}

pub async fn fetch_pr_review(
    project_path: &str,
    pr_url_or_number: &str,
    token: Option<String>,
) -> Result<GitDiffSummary, String> {
    let root = Path::new(project_path);
    let target = resolve_pr_target(root, pr_url_or_number)?;
    let tok = resolve_token(&target, token);

    let client = reqwest::Client::builder()
        .user_agent("vcode-reviewer/0.1")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let target_clone = target.clone();
    match target {
        ParsedPrTarget::GitHub {
            owner,
            repo,
            number,
        } => {
            let api_base = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
            let mut req = client.get(&api_base);
            if let Some(t) = &tok {
                req = req.bearer_auth(t);
            }
            let api_resp = match req.send().await {
                Ok(r) => r.error_for_status().map_err(|e| format!("GitHub API error: {e}")),
                Err(e) => Err(format!("GitHub request failed: {e}")),
            };
            let pr_json: serde_json::Value = match api_resp {
                Ok(r) => r
                    .json()
                    .await
                    .map_err(|e| format!("Invalid GitHub JSON: {e}"))?,
                Err(api_err) => {
                    let pr_url = format!("https://github.com/{owner}/{repo}/pull/{number}");
                    match fetch_pr_via_cli(root, &target_clone, "github", number, pr_url, tok.as_deref()) {
                        Ok(cli_diff) => return Ok(cli_diff),
                        Err(cli_err) => {
                            return Err(format!(
                                "{api_err} (gh fallback: {cli_err}; set a token in Settings if this is a private repo)"
                            ));
                        }
                    }
                }
            };

            let title = pr_json["title"].as_str().unwrap_or("PR").to_string();
            let url = pr_json["html_url"].as_str().unwrap_or("").to_string();
            let base_branch = pr_json["base"]["ref"].as_str().unwrap_or("main").to_string();
            let head_branch = pr_json["head"]["ref"].as_str().unwrap_or("").to_string();
            let head_sha = pr_json["head"]["sha"].as_str().unwrap_or("").to_string();

            let mut files_req = client.get(format!("{api_base}/files?per_page=100"));
            if let Some(t) = &tok {
                files_req = files_req.bearer_auth(t);
            }
            let files_json: Vec<serde_json::Value> = files_req
                .send()
                .await
                .map_err(|e| format!("GitHub files request failed: {e}"))?
                .error_for_status()
                .map_err(|e| format!("GitHub files API error: {e}"))?
                .json()
                .await
                .map_err(|e| format!("Invalid GitHub files JSON: {e}"))?;

            let checkout_root = root.to_path_buf();
            let checkout_hb = head_branch.clone();
            let checkout_sha = head_sha.clone();
            let checkout_tok = tok.clone();
            let checkout_target = target_clone.clone();
            let checkout_dir = tauri::async_runtime::spawn_blocking(move || {
                checkout_pr_to_temp_dir(
                    &checkout_root,
                    &checkout_target,
                    Some(&checkout_hb),
                    Some(&checkout_sha),
                    checkout_tok.as_deref(),
                )
            })
            .await
            .map_err(|e| format!("PR checkout task failed: {e}"))??;

            let mut files = Vec::new();
            for f in files_json {
                let rel = f["filename"].as_str().unwrap_or("").to_string();
                let status = match f["status"].as_str().unwrap_or("modified") {
                    "added" => DiffStatus::Added,
                    "removed" => DiffStatus::Deleted,
                    _ => DiffStatus::Modified,
                };
                let patch = f["patch"].as_str().unwrap_or("");
                let (hunks, p_add, p_del) = parse_hunks_from_patch(patch);
                let additions = f["additions"].as_u64().map(|n| n as usize).unwrap_or(p_add);
                let deletions = f["deletions"].as_u64().map(|n| n as usize).unwrap_or(p_del);
                files.push(FileDiff {
                    abs_path: checkout_dir.join(&rel).to_string_lossy().into_owned(),
                    file_path: rel,
                    status,
                    additions,
                    deletions,
                    hunks,
                });
            }

            Ok(GitDiffSummary {
                branch: head_branch.clone(),
                base_ref: base_branch.clone(),
                files,
                pr: Some(PrInfo {
                    provider: "github".to_string(),
                    number,
                    title,
                    url,
                    base_branch,
                    head_branch,
                    head_sha,
                    repo_name: target_clone.repo_name(),
                    checkout_path: Some(checkout_dir.to_string_lossy().into_owned()),
                }),
            })
        }
        ParsedPrTarget::GitLab {
            host,
            project_path: proj,
            iid,
        } => {
            let encoded_proj = proj.replace('/', "%2F");
            let api_base =
                format!("https://{host}/api/v4/projects/{encoded_proj}/merge_requests/{iid}");
            let mut req = client.get(&api_base);
            if let Some(t) = &tok {
                req = req.header("PRIVATE-TOKEN", t);
            }
            let mr_resp = match req.send().await {
                Ok(r) => r.error_for_status().map_err(|e| format!("GitLab API error: {e}")),
                Err(e) => Err(format!("GitLab request failed: {e}")),
            };
            let mr_json: serde_json::Value = match mr_resp {
                Ok(r) => r
                    .json()
                    .await
                    .map_err(|e| format!("Invalid GitLab JSON: {e}"))?,
                Err(api_err) => {
                    let mr_url = format!("https://{host}/{proj}/-/merge_requests/{iid}");
                    match fetch_pr_via_cli(root, &target_clone, "gitlab", iid, mr_url, tok.as_deref()) {
                        Ok(cli_diff) => return Ok(cli_diff),
                        Err(cli_err) => {
                            return Err(format!(
                                "{api_err} (glab fallback: {cli_err}; set a token in Settings if this is a private repo)"
                            ));
                        }
                    }
                }
            };

            let title = mr_json["title"].as_str().unwrap_or("MR").to_string();
            let url = mr_json["web_url"].as_str().unwrap_or("").to_string();
            let base_branch = mr_json["target_branch"]
                .as_str()
                .unwrap_or("main")
                .to_string();
            let head_branch = mr_json["source_branch"].as_str().unwrap_or("").to_string();
            let head_sha = mr_json["sha"].as_str().unwrap_or("").to_string();

            let mut ch_req = client.get(format!("{api_base}/changes"));
            if let Some(t) = &tok {
                ch_req = ch_req.header("PRIVATE-TOKEN", t);
            }
            let ch_json: serde_json::Value = ch_req
                .send()
                .await
                .map_err(|e| format!("GitLab changes request failed: {e}"))?
                .error_for_status()
                .map_err(|e| format!("GitLab changes API error: {e}"))?
                .json()
                .await
                .map_err(|e| format!("Invalid GitLab changes JSON: {e}"))?;

            let checkout_root = root.to_path_buf();
            let checkout_hb = head_branch.clone();
            let checkout_sha = head_sha.clone();
            let checkout_tok = tok.clone();
            let checkout_target = target_clone.clone();
            let checkout_dir = tauri::async_runtime::spawn_blocking(move || {
                checkout_pr_to_temp_dir(
                    &checkout_root,
                    &checkout_target,
                    Some(&checkout_hb),
                    Some(&checkout_sha),
                    checkout_tok.as_deref(),
                )
            })
            .await
            .map_err(|e| format!("PR checkout task failed: {e}"))??;

            let mut files = Vec::new();
            if let Some(arr) = ch_json["changes"].as_array() {
                for c in arr {
                    let rel = c["new_path"].as_str().unwrap_or("").to_string();
                    let status = if c["new_file"].as_bool().unwrap_or(false) {
                        DiffStatus::Added
                    } else if c["deleted_file"].as_bool().unwrap_or(false) {
                        DiffStatus::Deleted
                    } else {
                        DiffStatus::Modified
                    };
                    let patch = c["diff"].as_str().unwrap_or("");
                    let (hunks, additions, deletions) = parse_hunks_from_patch(patch);
                    files.push(FileDiff {
                        abs_path: checkout_dir.join(&rel).to_string_lossy().into_owned(),
                        file_path: rel,
                        status,
                        additions,
                        deletions,
                        hunks,
                    });
                }
            }

            Ok(GitDiffSummary {
                branch: head_branch.clone(),
                base_ref: base_branch.clone(),
                files,
                pr: Some(PrInfo {
                    provider: "gitlab".to_string(),
                    number: iid,
                    title,
                    url,
                    base_branch,
                    head_branch,
                    head_sha,
                    repo_name: target_clone.repo_name(),
                    checkout_path: Some(checkout_dir.to_string_lossy().into_owned()),
                }),
            })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrListItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    #[serde(default)]
    pub repo_name: String,
    pub head_branch: String,
    pub base_branch: String,
    pub draft: bool,
    pub is_author: bool,
    pub is_reviewer: bool,
    pub is_assigned: bool,
    #[serde(default)]
    pub is_involved: bool,
}

fn extract_repo_name_from_pr_json(pr: &serde_json::Value, default_repo: &str) -> String {
    if let Some(nwo) = pr
        .get("repository")
        .and_then(|r| r.get("nameWithOwner"))
        .and_then(|v| v.as_str())
    {
        return nwo.to_string();
    }
    if let Some(repo_url) = pr.get("repository_url").and_then(|v| v.as_str()) {
        if let Some((_, rest)) = repo_url.split_once("/repos/") {
            return rest.trim_end_matches('/').to_string();
        }
    }
    if let Some(html_url) = pr.get("html_url").or_else(|| pr.get("url")).and_then(|v| v.as_str()) {
        if let Some((_, rest)) = html_url.split_once("github.com/") {
            let parts: Vec<&str> = rest.split('/').collect();
            if parts.len() >= 2 {
                return format!("{}/{}", parts[0], parts[1]);
            }
        }
    }
    default_repo.to_string()
}

fn parse_github_pr_list_with_repo(
    items: &[serde_json::Value],
    current_user: Option<&str>,
    default_repo: &str,
    mark_all_involved: bool,
) -> Vec<PrListItem> {
    items
        .iter()
        .filter_map(|pr| {
            let number = pr.get("number").and_then(|v| v.as_u64())?;
            let title = pr.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let url = pr
                .get("html_url")
                .or_else(|| pr.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let author = pr
                .get("user")
                .and_then(|u| u.get("login"))
                .or_else(|| pr.get("author").and_then(|u| u.get("login")))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let head_branch = pr
                .get("head")
                .and_then(|h| h.get("ref"))
                .or_else(|| pr.get("headRefName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let base_branch = pr
                .get("base")
                .and_then(|b| b.get("ref"))
                .or_else(|| pr.get("baseRefName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let draft = pr
                .get("draft")
                .or_else(|| pr.get("isDraft"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let is_author = current_user.is_some_and(|u| u.eq_ignore_ascii_case(&author));
            let is_reviewer = current_user.is_some_and(|u| {
                pr.get("requested_reviewers")
                    .or_else(|| pr.get("reviewRequests"))
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| {
                        arr.iter().any(|r| {
                            r.get("login")
                                .and_then(|l| l.as_str())
                                .is_some_and(|l| l.eq_ignore_ascii_case(u))
                        })
                    })
            });
            let is_assigned = current_user.is_some_and(|u| {
                pr.get("assignees")
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| {
                        arr.iter().any(|a| {
                            a.get("login")
                                .and_then(|l| l.as_str())
                                .is_some_and(|l| l.eq_ignore_ascii_case(u))
                        })
                    })
            });
            let repo_name = extract_repo_name_from_pr_json(pr, default_repo);
            let is_involved = mark_all_involved || is_author || is_reviewer || is_assigned;
            Some(PrListItem {
                number,
                title,
                url,
                author,
                repo_name,
                head_branch,
                base_branch,
                draft,
                is_author,
                is_reviewer,
                is_assigned,
                is_involved,
            })
        })
        .collect()
}

#[cfg(test)]
fn parse_github_pr_list(items: &[serde_json::Value], current_user: Option<&str>) -> Vec<PrListItem> {
    parse_github_pr_list_with_repo(items, current_user, "", false)
}

pub async fn list_involved_prs_all_repos(token: Option<String>) -> Result<Vec<PrListItem>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let tok = resolve_github_token(token);
    let mut current_user: Option<String> = None;
    if let Some(t) = tok.as_deref() {
        if let Ok(u_resp) = client
            .get("https://api.github.com/user")
            .header("User-Agent", "vcode-app")
            .header("Authorization", format!("Bearer {t}"))
            .send()
            .await
        {
            if u_resp.status().is_success() {
                if let Ok(uj) = u_resp.json::<serde_json::Value>().await {
                    current_user = uj.get("login").and_then(|v| v.as_str()).map(str::to_string);
                }
            }
        }
    }

    if let Some(t) = tok.as_deref() {
        // Fetch involved PRs and review-requested PRs so we can accurately tag review requests
        let mut review_urls = std::collections::HashSet::<String>::new();
        if let Ok(r_resp) = client
            .get("https://api.github.com/search/issues?q=is:open+is:pr+review-requested:@me&sort=updated&order=desc&per_page=50")
            .header("User-Agent", "vcode-app")
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {t}"))
            .send()
            .await
        {
            if r_resp.status().is_success() {
                if let Ok(rj) = r_resp.json::<serde_json::Value>().await {
                    if let Some(arr) = rj.get("items").and_then(|v| v.as_array()) {
                        for it in arr {
                            if let Some(u) = it.get("html_url").and_then(|v| v.as_str()) {
                                review_urls.insert(u.to_string());
                            }
                        }
                    }
                }
            }
        }

        if let Ok(resp) = client
            .get("https://api.github.com/search/issues?q=is:open+is:pr+involves:@me&sort=updated&order=desc&per_page=50")
            .header("User-Agent", "vcode-app")
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {t}"))
            .send()
            .await
        {
            if resp.status().is_success() {
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse GitHub PR search: {e}"))?;
                let items = body
                    .get("items")
                    .and_then(|v| v.as_array())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let mut parsed =
                    parse_github_pr_list_with_repo(items, current_user.as_deref(), "", true);
                for pr in &mut parsed {
                    if review_urls.contains(&pr.url) {
                        pr.is_reviewer = true;
                    }
                }
                return Ok(parsed);
            }
        }
    }

    // Fallback to gh CLI
    let gh_bin = find_cli_binary("gh");
    if current_user.is_none() {
        if let Ok(u_out) = Command::new(&gh_bin).args(["api", "user", "--jq", ".login"]).output() {
            if u_out.status.success() {
                let u = String::from_utf8_lossy(&u_out.stdout).trim().to_string();
                if !u.is_empty() {
                    current_user = Some(u);
                }
            }
        }
    }
    let out = Command::new(&gh_bin)
        .args([
            "search",
            "prs",
            "--involves",
            "@me",
            "--state",
            "open",
            "--limit",
            "50",
            "--json",
            "number,title,url,author,repository,isDraft,assignees",
        ])
        .output()
        .map_err(|e| format!("Failed to run gh search prs: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Failed to list PRs across repositories (configure a GitHub token in Settings or sign in with `gh auth login`): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let items: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Failed to parse gh search prs JSON: {e}"))?;
    Ok(parse_github_pr_list_with_repo(
        &items,
        current_user.as_deref(),
        "",
        true,
    ))
}

pub async fn list_repo_prs(
    project_path: &str,
    token: Option<String>,
) -> Result<Vec<PrListItem>, String> {
    let root = Path::new(project_path);
    let (_, host, repo_path) = parse_remote_repo(root)
        .ok_or_else(|| "No GitHub/GitLab origin remote found for this project.".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    if host.contains("github.com") {
        let mut parts = repo_path.splitn(2, '/');
        let owner = parts.next().unwrap_or("");
        let repo = parts.next().unwrap_or("");
        if owner.is_empty() || repo.is_empty() {
            return Err(format!("Invalid GitHub repository path: {repo_path}"));
        }
        let tok = resolve_github_token(token);
        let mut current_user: Option<String> = None;
        if let Some(t) = tok.as_deref() {
            if let Ok(u_resp) = client
                .get("https://api.github.com/user")
                .header("User-Agent", "vcode-app")
                .header("Authorization", format!("Bearer {t}"))
                .send()
                .await
            {
                if u_resp.status().is_success() {
                    if let Ok(uj) = u_resp.json::<serde_json::Value>().await {
                        current_user = uj.get("login").and_then(|v| v.as_str()).map(str::to_string);
                    }
                }
            }
        }

        let api_url = format!("https://api.github.com/repos/{owner}/{repo}/pulls?state=open&per_page=50");
        let mut req = client
            .get(&api_url)
            .header("User-Agent", "vcode-app")
            .header("Accept", "application/vnd.github+json");
        if let Some(t) = tok.as_deref() {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let api_res = req.send().await;
        if let Ok(resp) = api_res {
            if resp.status().is_success() {
                let items: Vec<serde_json::Value> = resp
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse GitHub PR list: {e}"))?;
                return Ok(parse_github_pr_list_with_repo(
                    &items,
                    current_user.as_deref(),
                    &repo_path,
                    false,
                ));
            }
        }

        // Fallback to gh CLI
        let gh_bin = find_cli_binary("gh");
        if current_user.is_none() {
            if let Ok(u_out) = Command::new(&gh_bin).args(["api", "user", "--jq", ".login"]).output() {
                if u_out.status.success() {
                    let u = String::from_utf8_lossy(&u_out.stdout).trim().to_string();
                    if !u.is_empty() {
                        current_user = Some(u);
                    }
                }
            }
        }
        let out = Command::new(&gh_bin)
            .args([
                "pr",
                "list",
                "--repo",
                &format!("{owner}/{repo}"),
                "--state",
                "open",
                "--limit",
                "50",
                "--json",
                "number,title,url,author,headRefName,baseRefName,isDraft,reviewRequests,assignees",
            ])
            .output()
            .map_err(|e| format!("Failed to run gh pr list: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "gh pr list failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let items: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("Failed to parse gh pr list JSON: {e}"))?;
        return Ok(parse_github_pr_list_with_repo(
            &items,
            current_user.as_deref(),
            &repo_path,
            false,
        ));
    }

    // GitLab
    let tok = token
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("GITLAB_TOKEN").ok().filter(|s| !s.trim().is_empty()))
        .or_else(|| resolve_cli_token("glab", &["config", "get", "token"]));
    let mut current_user: Option<String> = None;
    if let Some(t) = tok.as_deref() {
        if let Ok(u_resp) = client
            .get(format!("https://{host}/api/v4/user"))
            .header("PRIVATE-TOKEN", t)
            .send()
            .await
        {
            if u_resp.status().is_success() {
                if let Ok(uj) = u_resp.json::<serde_json::Value>().await {
                    current_user = uj.get("username").and_then(|v| v.as_str()).map(str::to_string);
                }
            }
        }
    }
    let encoded = repo_path.replace('/', "%2F");
    let mr_url = format!("https://{host}/api/v4/projects/{encoded}/merge_requests?state=opened&per_page=50");
    let mut req = client.get(&mr_url).header("User-Agent", "vcode-app");
    if let Some(t) = tok.as_deref() {
        req = req.header("PRIVATE-TOKEN", t);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Failed to fetch GitLab MRs: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitLab MR list failed (HTTP {})", resp.status()));
    }
    let items: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitLab MR list: {e}"))?;
    let out = items
        .iter()
        .filter_map(|mr| {
            let number = mr.get("iid").and_then(|v| v.as_u64())?;
            let title = mr.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let url = mr.get("web_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let author = mr
                .get("author")
                .and_then(|u| u.get("username"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let head_branch = mr.get("source_branch").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let base_branch = mr.get("target_branch").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let draft = mr.get("draft").or_else(|| mr.get("work_in_progress")).and_then(|v| v.as_bool()).unwrap_or(false);
            let is_author = current_user.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(&author));
            let is_reviewer = current_user.as_deref().is_some_and(|u| {
                mr.get("reviewers").and_then(|v| v.as_array()).is_some_and(|arr| {
                    arr.iter().any(|r| r.get("username").and_then(|l| l.as_str()).is_some_and(|l| l.eq_ignore_ascii_case(u)))
                })
            });
            let is_assigned = current_user.as_deref().is_some_and(|u| {
                mr.get("assignees").and_then(|v| v.as_array()).is_some_and(|arr| {
                    arr.iter().any(|a| a.get("username").and_then(|l| l.as_str()).is_some_and(|l| l.eq_ignore_ascii_case(u)))
                })
            });
            Some(PrListItem {
                number,
                title,
                url,
                author,
                repo_name: repo_path.clone(),
                head_branch,
                base_branch,
                draft,
                is_author,
                is_reviewer,
                is_assigned,
                is_involved: is_author || is_reviewer || is_assigned,
            })
        })
        .collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unified_diff_with_line_numbers() {
        let raw = "\
diff --git a/src/app.py b/src/app.py
index 111..222 100644
--- a/src/app.py
+++ b/src/app.py
@@ -10,3 +10,4 @@
 def hello():
-    return 1
+    x = 2
+    return x
";
        let files = parse_unified_diff(Path::new("/repo"), raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "src/app.py");
        assert_eq!(files[0].additions, 2);
        assert_eq!(files[0].deletions, 1);
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].new_start, 10);
        assert_eq!(files[0].hunks[0].lines.len(), 4);
    }

    #[test]
    fn truncates_huge_diffs_while_preserving_counts() {
        let mut patch = String::from("@@ -1,1 +1,35000 @@\n");
        for i in 0..35_000 {
            patch.push_str(&format!("+line {i}\n"));
        }
        let (hunks, adds, dels) = parse_hunks_from_patch(&patch);
        assert_eq!(adds, 35_000);
        assert_eq!(dels, 0);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines.len(), MAX_DIFF_LINES_PER_FILE + 1);
    }

    #[test]
    fn formats_pr_checkout_slug_repo_name_and_https_token() {
        let gh = ParsedPrTarget::GitHub {
            owner: "yeusin".to_string(),
            repo: "vcode".to_string(),
            number: 42,
        };
        assert_eq!(gh.repo_name(), "vcode");
        assert_eq!(gh.checkout_slug(), "yeusin-vcode-pr-42");
        assert_eq!(gh.pr_ref(), "refs/pull/42/head");
        assert_eq!(
            with_https_token("https://github.com/yeusin/vcode.git", Some("tok"), false),
            "https://x-access-token:tok@github.com/yeusin/vcode.git"
        );

        let gl = ParsedPrTarget::GitLab {
            host: "gitlab.com".to_string(),
            project_path: "org/sub/my-app".to_string(),
            iid: 7,
        };
        assert_eq!(gl.repo_name(), "my-app");
        assert_eq!(gl.checkout_slug(), "org-sub-my-app-mr-7");
        assert_eq!(gl.pr_ref(), "refs/merge-requests/7/head");
        assert_eq!(
            with_https_token("https://gitlab.com/org/sub/my-app.git", Some("tok"), true),
            "https://oauth2:tok@gitlab.com/org/sub/my-app.git"
        );
    }

    #[test]
    fn parses_github_repo_urls_and_shorthand() {
        assert_eq!(
            parse_github_repo_url("https://github.com/yeusin/vcode").unwrap(),
            (
                "yeusin".to_string(),
                "vcode".to_string(),
                "https://github.com/yeusin/vcode.git".to_string()
            )
        );
        assert_eq!(
            parse_github_repo_url("git@github.com:yeusin/vcode.git").unwrap(),
            (
                "yeusin".to_string(),
                "vcode".to_string(),
                "https://github.com/yeusin/vcode.git".to_string()
            )
        );
        assert_eq!(
            parse_github_repo_url("yeusin/vcode").unwrap(),
            (
                "yeusin".to_string(),
                "vcode".to_string(),
                "https://github.com/yeusin/vcode.git".to_string()
            )
        );
    }

    #[test]
    fn parses_github_pr_list_with_involvement_flags() {
        let raw = serde_json::json!([
            {
                "number": 42,
                "title": "Add PR selector",
                "html_url": "https://github.com/yeusin/vcode/pull/42",
                "user": { "login": "alice" },
                "head": { "ref": "feat/pr-list" },
                "base": { "ref": "main" },
                "draft": false,
                "requested_reviewers": [{ "login": "ykim" }],
                "assignees": []
            },
            {
                "number": 43,
                "title": "My own PR",
                "html_url": "https://github.com/yeusin/vcode/pull/43",
                "user": { "login": "ykim" },
                "head": { "ref": "fix/bug" },
                "base": { "ref": "main" },
                "draft": true,
                "requested_reviewers": [],
                "assignees": [{ "login": "ykim" }]
            }
        ]);
        let items = parse_github_pr_list(raw.as_array().unwrap(), Some("ykim"));
        assert_eq!(items.len(), 2);
        assert!(items[0].is_reviewer && !items[0].is_author);
        assert!(items[1].is_author && items[1].is_assigned && items[1].draft);
    }
}
