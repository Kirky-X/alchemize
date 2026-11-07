// SPDX-License-Identifier: Apache-2.0
//! Post-conversion LaTeX sanity rules for markdown output.
//!
//! Three pure checks (no IO): math delimiter pairing, environment closure,
//! orphan `$$` fences. Converters attach the findings to
//! `metadata.latex_warnings` so degraded math survives visibly instead of
//! silently rendering broken.

/// A single validation finding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LatexWarning {
    /// Rule that fired.
    pub rule: &'static str,
    /// Human-readable detail (line number when known).
    pub detail: String,
}

/// Run all three rules; returns findings (empty = clean).
pub fn validate_markdown_math(md: &str) -> Vec<LatexWarning> {
    let mut warnings = Vec::new();
    warnings.extend(check_delimiter_parity(md));
    warnings.extend(check_environment_closure(md));
    warnings.extend(check_orphan_display_fences(md));
    warnings
}

/// Count unescaped `$` outside `$$` pairs: an odd count means an unclosed
/// inline math span.
pub fn check_delimiter_parity(md: &str) -> Vec<LatexWarning> {
    let mut singles = 0usize;
    let chars: Vec<char> = md.chars().collect();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                i += 2;
                continue;
            }
            '\n' => {
                line += 1;
                i += 1;
                continue;
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '$' => {
                i += 2; // display delimiter counted by its own rule
            }
            '$' => {
                singles += 1;
                i += 1;
            }
            '`' => {
                // Skip fenced code blocks: raw `$` inside code is literal.
                let fence_len = if i + 2 < chars.len() && chars[i + 1] == '`' && chars[i + 2] == '`'
                {
                    3
                } else {
                    i += 1;
                    continue;
                };
                i += fence_len;
                // consume to end of line (info string), then to closing fence
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                while i < chars.len() {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    if chars[i] == '`' && md_fence_at(&chars, i) >= fence_len {
                        i += fence_len;
                        break;
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    if singles % 2 != 0 {
        vec![LatexWarning {
            rule: "delimiter_parity",
            detail: format!(
                "odd number of unescaped single `$` ({singles}); an inline math span is unclosed"
            ),
        }]
    } else {
        Vec::new()
    }
}

/// Number of consecutive backticks at `i`.
fn md_fence_at(chars: &[char], i: usize) -> usize {
    let mut n = 0;
    while i + n < chars.len() && chars[i + n] == '`' {
        n += 1;
    }
    n
}

/// `\begin{env}` / `\end{env}` must balance per environment name.
pub fn check_environment_closure(md: &str) -> Vec<LatexWarning> {
    use std::collections::BTreeMap;
    let mut depth: BTreeMap<String, i32> = BTreeMap::new();
    let mut rest = md;
    while let Some(pos) = rest.find("\\begin{") {
        let after = &rest[pos + "\\begin{".len()..];
        if let Some(end) = after.find('}') {
            let env = &after[..end];
            *depth.entry(env.to_string()).or_insert(0) += 1;
            rest = &after[end + 1..];
            continue;
        }
        break;
    }
    let mut rest = md;
    while let Some(pos) = rest.find("\\end{") {
        let after = &rest[pos + "\\end{".len()..];
        if let Some(end) = after.find('}') {
            let env = &after[..end];
            *depth.entry(env.to_string()).or_insert(0) -= 1;
            rest = &after[end + 1..];
            continue;
        }
        break;
    }
    depth
        .into_iter()
        .filter(|(_, d)| *d != 0)
        .map(|(env, d)| LatexWarning {
            rule: "environment_closure",
            detail: format!("environment `{env}` unbalanced (delta {d:+})"),
        })
        .collect()
}

/// A `$$` fence must pair with another `$$`; a lone one on its own line with
/// no partner anywhere is an orphan.
pub fn check_orphan_display_fences(md: &str) -> Vec<LatexWarning> {
    let count = count_unescaped_display(md);
    if count % 2 != 0 {
        vec![LatexWarning {
            rule: "orphan_display_fence",
            detail: format!("odd number of `$$` fences ({count})"),
        }]
    } else {
        Vec::new()
    }
}

fn count_unescaped_display(md: &str) -> usize {
    let chars: Vec<char> = md.chars().collect();
    let mut i = 0usize;
    let mut count = 0usize;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '$' {
            count += 1;
            i += 2;
            continue;
        }
        i += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_markdown_no_warnings() {
        let md = "text $a+b$ and\n\n$$\nc^2\n$$\n";
        assert!(validate_markdown_math(md).is_empty());
    }

    #[test]
    fn odd_single_dollar_flagged() {
        let md = "broken $unclosed here";
        let w = validate_markdown_math(md);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].rule, "delimiter_parity");
    }

    #[test]
    fn dollar_in_code_block_not_counted() {
        let md = "```bash\necho $PATH $\n```\n";
        assert!(check_delimiter_parity(md).is_empty());
    }

    #[test]
    fn unclosed_environment_flagged() {
        let md = "$$\n\\begin{align}\nx = 1\n$$\n";
        let w = check_environment_closure(md);
        assert_eq!(w.len(), 1);
        assert!(w[0].detail.contains("align"));
    }

    #[test]
    fn balanced_environments_clean() {
        let md = "\\begin{equation} E \\end{equation}";
        assert!(check_environment_closure(md).is_empty());
    }

    #[test]
    fn orphan_double_dollar_flagged() {
        let md = "intro\n$$\nx\n";
        let w = check_orphan_display_fences(md);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].rule, "orphan_display_fence");
    }

    #[test]
    fn escaped_dollars_ignored() {
        let md = r"cost is \$5 and \$6";
        assert!(check_delimiter_parity(md).is_empty());
    }
}
