use clap::ValueEnum;

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Component {
    Editor,
    Terminal,
    Browser,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LaunchSelection {
    pub editor: bool,
    pub terminal: bool,
    pub browser: bool,
}

impl LaunchSelection {
    pub const fn all() -> Self {
        Self {
            editor: true,
            terminal: true,
            browser: true,
        }
    }

    pub fn from_flags(only: &[Component], editor: bool, terminal: bool, browser: bool) -> Self {
        if only.is_empty() && !editor && !terminal && !browser {
            return Self::all();
        }
        let mut sel = Self {
            editor,
            terminal,
            browser,
        };
        for c in only {
            match c {
                Component::Editor => sel.editor = true,
                Component::Terminal => sel.terminal = true,
                Component::Browser => sel.browser = true,
            }
        }
        sel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_flags_selects_everything() {
        let sel = LaunchSelection::from_flags(&[], false, false, false);
        assert_eq!(sel, LaunchSelection::all());
    }

    #[test]
    fn short_editor_flag_excludes_others() {
        let sel = LaunchSelection::from_flags(&[], true, false, false);
        assert!(sel.editor);
        assert!(!sel.terminal);
        assert!(!sel.browser);
    }

    #[test]
    fn only_editor_equivalent_to_short_flag() {
        let sel = LaunchSelection::from_flags(&[Component::Editor], false, false, false);
        assert!(sel.editor);
        assert!(!sel.terminal);
        assert!(!sel.browser);
    }

    #[test]
    fn short_flags_combine() {
        let sel = LaunchSelection::from_flags(&[], true, false, true);
        assert!(sel.editor);
        assert!(!sel.terminal);
        assert!(sel.browser);
    }

    #[test]
    fn only_and_short_flags_take_union() {
        let sel = LaunchSelection::from_flags(&[Component::Browser], true, false, false);
        assert!(sel.editor);
        assert!(!sel.terminal);
        assert!(sel.browser);
    }

    #[test]
    fn duplicate_only_is_idempotent() {
        let sel = LaunchSelection::from_flags(
            &[Component::Editor, Component::Editor],
            false,
            false,
            false,
        );
        assert_eq!(
            sel,
            LaunchSelection {
                editor: true,
                terminal: false,
                browser: false,
            }
        );
    }
}
