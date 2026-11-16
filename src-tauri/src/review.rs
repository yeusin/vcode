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

pub fn parse_hunks_from_patch(patch: &str) -> (Vec<DiffHunk>, usize, usize) {
    let mut hunks = Vec::new();
    let mut current: Option<DiffHunk> = None;
    let mut old_ln = 0usize;
    let mut new_ln = 0usize;
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for raw in patch.lines() {
        if raw.starts_with("@@ ") {
            if let Some(h) = current.take() {
                hunks.push(h);
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
        let Some(h) = current.as_mut() else {
            continue;
        };
        if let Some(text) = raw.strip_prefix('+') {
            additions += 1;
            h.lines.push(DiffLine {
                kind: "add".to_string(),
                old_line: None,
                new_line: Some(new_ln),
                text: text.to_string(),
            });
            new_ln += 1;
        } else if let Some(text) = raw.strip_prefix('-') {
            deletions += 1;
            h.lines.push(DiffLine {
                kind: "del".to_string(),
                old_line: Some(old_ln),
                new_line: None,
                text: text.to_string(),
            });
            old_ln += 1;
        } else if let Some(text) = raw.strip_prefix(' ') {
            h.lines.push(DiffLine {
                kind: "context".to_string(),
                old_line: Some(old_ln),
                new_line: Some(new_ln),
                text: text.to_string(),
            });
            old_ln += 1;
            new_ln += 1;
        }
    }
    if let Some(h) = current {
        hunks.push(h);
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

fn resolve_token(target: &ParsedPrTarget, explicit: Option<String>) -> Option<String> {
    if let Some(t) = explicit.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        return Some(t);
    }
    match target {
        ParsedPrTarget::GitHub { .. } => std::env::var("GITHUB_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("GH_TOKEN").ok().filter(|s| !s.trim().is_empty()))
            .or_else(|| resolve_cli_token("gh", &["auth", "token"])),
        ParsedPrTarget::GitLab { .. } => std::env::var("GITLAB_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| resolve_cli_token("glab", &["config", "get", "token"])),
    }
}

fn fetch_pr_via_cli(
    root: &Path,
    provider: &str,
    number: u64,
    url: String,
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
    let diff_out = String::from_utf8_lossy(&out.stdout).into_owned();
    let files = parse_unified_diff(root, &diff_out);
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
                    match fetch_pr_via_cli(root, "github", number, pr_url) {
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
                    abs_path: root.join(&rel).to_string_lossy().into_owned(),
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
                    match fetch_pr_via_cli(root, "gitlab", iid, mr_url) {
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
                        abs_path: root.join(&rel).to_string_lossy().into_owned(),
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
                }),
            })
        }
    }
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
}
