use unicode_script::Script;

fn is_strong(script: Script) -> bool {
    !matches!(script, Script::Common | Script::Inherited)
}

pub(super) fn resolve_prior_or_next_strong(scripts: &[Script]) -> Vec<Script> {
    let mut resolved = Vec::with_capacity(scripts.len());
    let mut previous = None;
    for script in scripts.iter().copied() {
        if is_strong(script) {
            previous = Some(script);
            resolved.push(Some(script));
        } else {
            resolved.push(previous);
        }
    }

    let mut following = None;
    for index in (0..scripts.len()).rev() {
        let script = scripts[index];
        if is_strong(script) {
            following = Some(script);
        } else if resolved[index].is_none() {
            resolved[index] = following.or(Some(Script::Latin));
        }
    }
    resolved
        .into_iter()
        .map(|script| script.unwrap_or(Script::Latin))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_only_runs_default_to_latin_in_linear_passes() {
        let scripts = vec![Script::Common; 14_999];
        assert!(
            resolve_prior_or_next_strong(&scripts)
                .into_iter()
                .all(|script| script == Script::Latin)
        );
    }

    #[test]
    fn common_scripts_prefer_a_prior_strong_script_then_a_following_one() {
        let scripts = [
            Script::Common,
            Script::Arabic,
            Script::Common,
            Script::Common,
            Script::Common,
            Script::Cyrillic,
        ];
        assert_eq!(
            resolve_prior_or_next_strong(&scripts),
            [
                Script::Arabic,
                Script::Arabic,
                Script::Arabic,
                Script::Arabic,
                Script::Arabic,
                Script::Cyrillic,
            ]
        );
    }
}
