//! Workbench customization precedence follows VS Code 1.95.0 colorThemeData.ts.
//! Native palette aliases remain approximate; unmapped surfaces stay visible.
use super::*;
use serde_json::Map;

impl Theme {
    /// Called on the theme worker; never walk or clone arbitrary JSON in rendering.
    pub(crate) fn customized(&self, settings: &crate::settings::Settings) -> Self {
        let mut theme = self.clone();
        let mut merged = Map::new();
        for layer in settings.extension_layers().iter() {
            if let Some(value) = layer.get("workbench.colorCustomizations") {
                if let Some(colors) = value.as_object() {
                    merge(&mut merged, colors);
                } else {
                    merged.clear();
                    theme.warnings.push("workbench.colorCustomizations must be an object; lower-scope customizations are not applied".into());
                }
            }
        }
        // All global values precede all matching theme blocks, even when a
        // global key occurs later in the source. Keep block insertion order.
        let mut effective = Map::new();
        collect(&mut effective, &merged, &mut theme.warnings);
        for (selector, value) in &merged {
            if !is_scope(selector) {
                continue;
            }
            if let Some(block) = value.as_object() {
                for name in scope_names(selector) {
                    if matches_theme(name, &self.settings_id) {
                        collect(&mut effective, block, &mut theme.warnings);
                    }
                }
            } else {
                notice(
                    &mut theme.warnings,
                    format!("Color customization {selector} must be an object"),
                );
            }
        }
        let mut colors = Map::new();
        for (key, value) in effective {
            let Some(value) = value.as_str() else {
                continue;
            };
            let color = if value == "default" {
                match palette_color(&self.defaults, &key) {
                    Some(color) => color,
                    None => {
                        colors.insert(key, Value::String(value.into()));
                        continue;
                    }
                }
            } else {
                match color_value(value, self.colors.background) {
                    Ok(color) => color,
                    Err(_) => {
                        notice(
                            &mut theme.warnings,
                            format!("Invalid color customization for {key}; theme color retained"),
                        );
                        continue;
                    }
                }
            };
            // Keep the original alpha string: the shared loader resolves the
            // final customized background before compositing other colors.
            let text = if value == "default" {
                hex(color)
            } else {
                value.into()
            };
            colors.insert(key, Value::String(text));
        }
        // Invalid entries were filtered independently; a bad value cannot
        // discard valid customizations or mutate the underlying theme.
        if let Err(error) = theme.apply_colors(&colors) {
            notice(
                &mut theme.warnings,
                format!("Color customizations could not be applied: {error:#}"),
            );
        }
        theme
    }
}

fn merge(target: &mut Map<String, Value>, source: &Map<String, Value>) {
    for (key, value) in source {
        if let (Some(Value::Object(previous)), Value::Object(next)) = (target.get_mut(key), value) {
            merge(previous, next);
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn collect(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
    warnings: &mut Vec<String>,
) {
    for (key, value) in source {
        if is_scope(key) {
            continue;
        }
        if value.is_string() {
            target.insert(key.clone(), value.clone());
        } else {
            notice(
                warnings,
                format!("Color customization {key} must be a color string"),
            );
        }
    }
}

fn notice(warnings: &mut Vec<String>, message: String) {
    if warnings.len() < 64 {
        warnings.push(message.chars().take(512).collect());
    } else if warnings.len() == 64 {
        warnings.push("Additional color customization notices omitted".into());
    }
}

fn is_scope(value: &str) -> bool {
    value.starts_with('[') && value.ends_with(']')
}
fn scope_names(value: &str) -> impl Iterator<Item = &str> {
    value
        .split('[')
        .skip(1)
        .filter_map(|part| part.split_once(']').map(|(name, _)| name))
}
fn matches_theme(pattern: &str, name: &str) -> bool {
    if pattern == name {
        return true;
    }
    match (pattern.strip_prefix('*'), pattern.strip_suffix('*')) {
        (Some(_), Some(_)) if pattern.len() >= 2 => name.contains(&pattern[1..pattern.len() - 1]),
        (Some(suffix), _) => name.ends_with(suffix),
        (_, Some(prefix)) => name.starts_with(prefix),
        _ => false,
    }
}
fn palette_color(colors: &Colors, key: &str) -> Option<Color> {
    Some(match key {
        "editor.background" => colors.background,
        "editor.foreground" => colors.foreground,
        "sideBar.background" | "editorWidget.background" => colors.panel,
        "editorLineNumber.foreground" => colors.muted,
        "focusBorder" | "textLink.foreground" => colors.accent,
        "editor.selectionBackground" => colors.selection,
        "editor.lineHighlightBackground" => colors.current_line,
        _ => return None,
    })
}
fn hex(color: Color) -> String {
    let Color::Rgb(red, green, blue) = color else {
        unreachable!("Native defaults are RGB")
    };
    format!("#{red:02x}{green:02x}{blue:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(layers: &[Value]) -> crate::settings::Settings {
        let root = tempfile::tempdir().unwrap();
        let paths: Vec<_> = layers
            .iter()
            .enumerate()
            .map(|(i, layer)| {
                let path = root.path().join(format!("{i}.json"));
                fs::write(&path, serde_json::to_vec(layer).unwrap()).unwrap();
                path
            })
            .collect();
        crate::settings::Settings::load(&paths).unwrap()
    }
    fn customized(value: Value) -> Theme {
        Theme::default().customized(&settings(&[json!({"workbench.colorCustomizations":value})]))
    }

    #[test]
    fn themed_blocks_override_globals_in_source_order_and_use_theme_id() {
        let base = Theme {
            name: "Display label".into(),
            settings_id: "Personal Dark".into(),
            ..Theme::default()
        };
        let result = base.customized(&settings(&[json!({"workbench.colorCustomizations":{
            "[Personal Dark]":{"editor.foreground":"#111111"},
            "editor.foreground":"#eeeeee",
            "[Other][*Dark]":{"editor.foreground":"#222222"},
            "[Personal*]":{"editor.selectionBackground":"#123456"},
            "[*sonal*]":{"editorLineNumber.foreground":"#654321"},
            "[Display label]":{"editor.foreground":"#ffffff"},
            "[Per*Dark]":{"editor.foreground":"#ffffff"}
        }})]));
        assert_eq!(result.colors.foreground, Color::Rgb(34, 34, 34));
        assert_eq!(result.colors.selection, Color::Rgb(18, 52, 86));
        assert_eq!(result.colors.muted, Color::Rgb(101, 67, 33));
        assert_eq!(base.colors.foreground, Theme::default().colors.foreground);
    }

    #[test]
    fn workspace_merges_theme_blocks_before_theme_specific_overrides_globals() {
        let result = Theme::default().customized(&settings(&[
            json!({"workbench.colorCustomizations":{
                "editor.foreground":"#111111",
                "[VSCLI Dark]":{"editor.foreground":"#222222", "editor.selectionBackground":"#333333"}
            }}),
            json!({"workbench.colorCustomizations":{
                "editor.foreground":"#444444",
                "[VSCLI Dark]":{"editor.selectionBackground":"#555555"}
            }})
        ]));
        assert_eq!(result.colors.foreground, Color::Rgb(34, 34, 34));
        assert_eq!(result.colors.selection, Color::Rgb(85, 85, 85));
    }

    #[test]
    fn alpha_uses_custom_background_and_independent_invalid_values_do_not_erase_valid_colors() {
        let result = customized(json!({
            "editor.background":"#ffffff",
            "editor.selectionBackground":"#0008",
            "editor.foreground":"#zzzzzz",
            "editorLineNumber.foreground":42,
            "focusBorder":"#123",
            "statusBar.background":"#abcdef"
        }));
        assert_eq!(result.colors.background, Color::Rgb(255, 255, 255));
        assert_eq!(result.colors.selection, Color::Rgb(119, 119, 119));
        assert_eq!(result.colors.foreground, Theme::default().colors.foreground);
        assert_eq!(result.colors.accent, Color::Rgb(17, 34, 51));
        for key in [
            "editor.foreground",
            "editorLineNumber.foreground",
            "statusBar.background",
        ] {
            assert!(
                result.warnings.iter().any(|warning| warning.contains(key)),
                "{key}"
            );
        }
    }

    #[test]
    fn default_bypasses_selected_theme_and_uses_builtin_light_or_dark_defaults() {
        for mut base in [Theme::default(), Theme::light()] {
            let expected = base.colors.background;
            base.colors.background = Color::Rgb(1, 2, 3);
            let result = base.customized(&settings(&[json!({"workbench.colorCustomizations":{
                "editor.background":"default"
            }})]));
            assert_eq!(result.colors.background, expected);
            assert_eq!(base.colors.background, Color::Rgb(1, 2, 3));
        }
    }

    #[test]
    fn removing_customizations_restores_base_and_malformed_winning_layer_masks_lower_layer() {
        let base = Theme::default();
        let custom =
            settings(&[json!({"workbench.colorCustomizations":{"editor.foreground":"#123456"}})]);
        assert_ne!(
            base.customized(&custom).colors.foreground,
            base.colors.foreground
        );
        assert_eq!(
            base.customized(&crate::settings::Settings::default())
                .colors
                .foreground,
            base.colors.foreground
        );
        let invalid = settings(&[
            json!({"workbench.colorCustomizations":{"editor.foreground":"#123456"}}),
            json!({"workbench.colorCustomizations":null}),
        ]);
        let result = base.customized(&invalid);
        assert_eq!(result.colors.foreground, base.colors.foreground);
        assert!(
            result
                .warnings
                .iter()
                .any(|message| message.contains("must be an object"))
        );
    }

    #[test]
    fn language_overrides_do_not_change_workbench_colors_and_notices_are_bounded() {
        let result = Theme::default().customized(&settings(&[json!({
            "[cpp]":{"workbench.colorCustomizations":{"editor.foreground":"#123456"}}
        })]));
        assert_eq!(result.colors.foreground, Theme::default().colors.foreground);
        let colors: Map<String, Value> = (0..200)
            .map(|i| (format!("invalid.{i}"), Value::Bool(true)))
            .collect();
        let result = customized(Value::Object(colors));
        assert_eq!(result.warnings.len(), 65);
        assert!(result.warnings.last().unwrap().contains("omitted"));
    }

    #[test]
    fn wildcard_matching_is_literal_except_at_edges_and_handles_unicode() {
        for pattern in ["*", "**", "*Dark*", "λ*", "*Dark", "λ Dark"] {
            assert!(matches_theme(pattern, "λ Dark"), "{pattern}");
        }
        for pattern in ["λ*Dark", "dark", "*Light*", "[λ Dark]"] {
            assert!(!matches_theme(pattern, "λ Dark"), "{pattern}");
        }
    }
}
