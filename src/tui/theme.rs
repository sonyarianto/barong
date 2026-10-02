use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub primary: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub muted: Color,
    pub code_theme: &'static str,
}

pub fn all_themes() -> Vec<(&'static str, &'static str)> {
    vec![
        ("dark", "default dark terminal"),
        ("light", "bright terminal"),
        ("barong", "Bali guardian red-gold"),
    ]
}

pub fn resolve(name: &str) -> Theme {
    match name.trim().to_lowercase().as_str() {
        "light" => Theme {
            name: "light".into(),
            primary: Color::Blue,
            accent: Color::Blue,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            muted: Color::Gray,
            code_theme: "InspiredGitHub",
        },
        "barong" => Theme {
            name: "barong".into(),
            primary: Color::Red,
            accent: Color::Yellow,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            muted: Color::Gray,
            code_theme: "base16-ocean.dark",
        },
        _ => Theme {
            name: "dark".into(),
            primary: Color::Green,
            accent: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            muted: Color::DarkGray,
            code_theme: "base16-ocean.dark",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_themes() {
        assert_eq!(resolve("dark").name, "dark");
        assert_eq!(resolve("LIGHT").name, "light");
        assert_eq!(resolve("barong").code_theme, "base16-ocean.dark");
        assert_eq!(resolve("unknown").name, "dark");
    }
}
