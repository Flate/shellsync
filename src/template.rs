//! Shell-neutral string templates used by env values and PATH entries.
//!
//! Syntax:
//!   `~` at the start (alone or followed by `/`)  -> the home directory
//!   `$NAME` or `${NAME}`                         -> value of an env var
//!   `$$`                                         -> a literal `$`
//! Any other `$` is kept literally.
//!
//! References are rendered as references in each shell's own syntax, so they
//! are resolved when the shell starts, not when the config is built.

#[derive(Debug, PartialEq)]
pub enum Seg {
    Lit(String),
    Var(String),
}

pub fn parse(s: &str) -> Vec<Seg> {
    let mut segs = Vec::new();
    let mut lit = String::new();
    let mut rest = s;

    if rest == "~" || rest.starts_with("~/") {
        segs.push(Seg::Var("HOME".into()));
        rest = &rest[1..];
    }

    while let Some(i) = rest.find('$') {
        lit.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        // (variable name, bytes of `after` consumed)
        let (var, consumed) = if after.starts_with('$') {
            lit.push('$');
            (None, 1)
        } else if let Some(body) = after.strip_prefix('{') {
            match body.find('}') {
                Some(end) if crate::config::is_env_name(&body[..end]) => {
                    (Some(&body[..end]), end + 2)
                }
                _ => {
                    lit.push('$');
                    (None, 0)
                }
            }
        } else {
            let len = after
                .char_indices()
                .take_while(|&(j, c)| {
                    c == '_' || c.is_ascii_alphabetic() || (j > 0 && c.is_ascii_digit())
                })
                .count();
            if len == 0 {
                lit.push('$');
            }
            ((len > 0).then(|| &after[..len]), len)
        };
        if let Some(name) = var {
            if !lit.is_empty() {
                segs.push(Seg::Lit(std::mem::take(&mut lit)));
            }
            segs.push(Seg::Var(name.to_string()));
        }
        rest = &after[consumed..];
    }
    lit.push_str(rest);
    if !lit.is_empty() {
        segs.push(Seg::Lit(lit));
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;
    use Seg::*;

    fn l(s: &str) -> Seg {
        Lit(s.into())
    }
    fn v(s: &str) -> Seg {
        Var(s.into())
    }

    #[test]
    fn parses_templates() {
        assert_eq!(parse(""), vec![]);
        assert_eq!(parse("plain"), vec![l("plain")]);
        assert_eq!(parse("~"), vec![v("HOME")]);
        assert_eq!(parse("~/go/bin"), vec![v("HOME"), l("/go/bin")]);
        assert_eq!(parse("~user/x"), vec![l("~user/x")]);
        assert_eq!(parse("a/~/b"), vec![l("a/~/b")]);
        assert_eq!(parse("$GOPATH/bin"), vec![v("GOPATH"), l("/bin")]);
        assert_eq!(parse("${A}x${B_2}"), vec![v("A"), l("x"), v("B_2")]);
        assert_eq!(parse("$A$B"), vec![v("A"), v("B")]);
        assert_eq!(parse("cost: $$5"), vec![l("cost: $5")]);
        assert_eq!(
            parse("$ $1 ${} ${bad-name} end$"),
            vec![l("$ $1 ${} ${bad-name} end$")]
        );
        assert_eq!(parse("ünï$X©"), vec![l("ünï"), v("X"), l("©")]);
    }
}
