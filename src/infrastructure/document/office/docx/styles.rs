// SPDX-License-Identifier: Apache-2.0
//! DOCX style cascade.
//!
//! OOXML resolves character formatting in this order:
//!   `docDefaults/rPrDefault` → `basedOn` chain (root first) → the style's own
//!   `rPr` → the run's direct `rPr`.
//!
//! Every step overrides the previous one; a `w:val="0"` toggle explicitly
//! clears an inherited flag, so the cascade tracks *explicit* values rather
//! than plain booleans.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::domain::document::Style;

/// A style as declared in `styles.xml`, before cascade resolution.
#[derive(Default, Clone)]
pub struct StyleDef {
    /// `<w:name w:val="…"/>` — human-readable style name.
    pub name: String,
    /// `<w:basedOn w:val="…"/>` — parent style id.
    pub based_on: Option<String>,
    /// `<w:pPr><w:outlineLvl w:val="N"/>` — heading depth (0-based).
    pub outline_lvl: Option<u8>,
    /// Explicit bold state (`None` = not specified, inherits).
    pub bold: Option<bool>,
    /// Explicit italic state.
    pub italic: Option<bool>,
    /// Font size in half-points (`<w:sz w:val="24"/>` = 12pt).
    pub sz_half_points: Option<u32>,
    /// Whether the style is a heading style (by id/name convention).
    pub is_heading: bool,
    /// Heading depth derived from the style name ("heading 2" → 2).
    pub heading_level: Option<u8>,
}

/// Resolved character formatting for one run.
///
/// Flags are three-state (`None` = unspecified): the cascade keeps explicit
/// `w:val="0"` toggles apart from "not set", so a derived style can clear an
/// inherited flag.
#[derive(Default, Clone, Copy, PartialEq)]
pub struct RunProps {
    /// Bold: `None` unspecified, `Some(true)` on, `Some(false)` off.
    pub bold: Option<bool>,
    /// Italic: same three states.
    pub italic: Option<bool>,
    /// Font size in points.
    pub size_pt: Option<f32>,
}

impl RunProps {
    /// Merge `other` (higher precedence) over `self`.
    pub fn merge(self, other: RunProps) -> RunProps {
        RunProps {
            bold: other.bold.or(self.bold),
            italic: other.italic.or(self.italic),
            size_pt: other.size_pt.or(self.size_pt),
        }
    }

    /// Convert to the public style bitset (unspecified = off).
    pub fn to_style(self) -> Style {
        Style {
            bold: self.bold.unwrap_or(false),
            italic: self.italic.unwrap_or(false),
            code: false,
            strike: false,
            size_pt: self.size_pt,
        }
    }
}

/// The full style cascade: defaults plus every declared style.
#[derive(Default)]
pub struct StyleTable {
    /// `docDefaults/rPrDefault` — the base every run inherits.
    pub defaults: RunProps,
    /// Declared paragraph styles by style id.
    pub styles: HashMap<String, StyleDef>,
    /// Declared character styles by style id (resolved for `w:rStyle`).
    pub char_styles: HashMap<String, StyleDef>,
}

impl StyleTable {
    /// Resolve a paragraph style id into (run props, outline level, heading).
    ///
    /// The `basedOn` chain is walked root-first so the most-derived style
    /// wins; each ancestor contributes only what its descendants leave unset.
    pub fn resolve_paragraph(&self, style_id: &str) -> (RunProps, Option<u8>, Option<u8>) {
        let mut props = self.defaults;
        let mut outline = None;
        let mut heading = None;
        let mut visited = Vec::new();
        self.walk_paragraph(
            style_id,
            &mut props,
            &mut outline,
            &mut heading,
            &mut visited,
        );
        (props, outline, heading)
    }

    fn walk_paragraph(
        &self,
        style_id: &str,
        props: &mut RunProps,
        outline: &mut Option<u8>,
        heading: &mut Option<u8>,
        visited: &mut Vec<String>,
    ) {
        if visited.iter().any(|s| s == style_id) {
            return; // cycle guard
        }
        visited.push(style_id.to_string());
        let Some(def) = self.styles.get(style_id) else {
            return;
        };
        // Parent first: the derived style overrides the base.
        if let Some(parent) = &def.based_on {
            self.walk_paragraph(parent, props, outline, heading, visited);
        }
        let own = RunProps {
            bold: def.bold,
            italic: def.italic,
            size_pt: def.sz_half_points.map(|h| h as f32 / 2.0),
        };
        *props = props.merge(own);
        if let Some(lvl) = def.outline_lvl {
            *outline = Some(lvl);
        }
        if let Some(lvl) = def.heading_level {
            *heading = Some(lvl);
        }
        if def.is_heading && heading.is_none() {
            *heading = Some(2);
        }
    }

    /// Resolve a character style id (`w:rStyle`) into run props.
    pub fn resolve_character(&self, style_id: &str) -> RunProps {
        let mut props = self.defaults;
        let mut visited = Vec::new();
        self.walk_character(style_id, &mut props, &mut visited);
        props
    }

    fn walk_character(&self, style_id: &str, props: &mut RunProps, visited: &mut Vec<String>) {
        if visited.iter().any(|s| s == style_id) {
            return;
        }
        visited.push(style_id.to_string());
        let Some(def) = self.char_styles.get(style_id) else {
            return;
        };
        if let Some(parent) = &def.based_on {
            self.walk_character(parent, props, visited);
        }
        let own = RunProps {
            bold: def.bold,
            italic: def.italic,
            size_pt: def.sz_half_points.map(|h| h as f32 / 2.0),
        };
        *props = props.merge(own);
    }
}

/// Style declaration kind.
#[derive(Clone, Copy, PartialEq)]
enum StyleKind {
    Paragraph,
    Character,
    Numbering,
}

/// Parse `styles.xml` into the style cascade table.
pub fn parse_styles(xml: &[u8]) -> StyleTable {
    let mut table = StyleTable::default();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    // Declaration state.
    let mut current: Option<(String, StyleDef, StyleKind)> = None;
    let mut in_doc_defaults = false;
    let mut in_rpr_default = false;
    let mut in_style_rpr = false;
    let mut in_name = false;
    let mut style_name = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "docDefaults" => in_doc_defaults = true,
                    "rPrDefault" if in_doc_defaults => in_rpr_default = true,
                    "style" => {
                        let mut id = String::new();
                        let mut kind = StyleKind::Paragraph;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                "w:styleId" => id = attr.value.clone().into_owned(),
                                "w:type" => {
                                    kind = match attr.value.as_ref() {
                                        "character" => StyleKind::Character,
                                        "numbering" => StyleKind::Numbering,
                                        _ => StyleKind::Paragraph,
                                    }
                                }
                                _ => {}
                            }
                        }
                        let mut def = StyleDef::default();
                        // Heading styles are named Heading1..9 / Titre1..9.
                        if id.starts_with("Heading")
                            || id.starts_with("heading")
                            || id.starts_with("Titre")
                            || id.starts_with("标题")
                        {
                            def.is_heading = true;
                        }
                        current = Some((id, def, kind));
                        style_name.clear();
                    }
                    "name" if current.is_some() => {
                        // <w:name w:val="heading 1"/> — value rides an attribute.
                        if let Some(v) = attr_value(&e, "w:val") {
                            style_name = v;
                        }
                        in_name = true;
                    }
                    "basedOn" if current.is_some() => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Some((_, def, _)) = &mut current
                        {
                            def.based_on = Some(v);
                        }
                    }
                    "outlineLvl" if current.is_some() => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Ok(lvl) = v.trim().parse::<u8>()
                            && let Some((_, def, _)) = &mut current
                        {
                            def.outline_lvl = Some(lvl);
                        }
                    }
                    "rPr" => {
                        if current.is_some() {
                            in_style_rpr = true;
                        }
                    }
                    "b" | "i" => {
                        let on = !has_attr_false(&e);
                        let key = local.clone();
                        if in_rpr_default {
                            if key == "b" {
                                table.defaults.bold = Some(on);
                            } else {
                                table.defaults.italic = Some(on);
                            }
                        } else if in_style_rpr && let Some((_, def, _)) = &mut current {
                            if key == "b" {
                                def.bold = Some(on);
                            } else {
                                def.italic = Some(on);
                            }
                        }
                    }
                    "sz" => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Ok(hp) = v.trim().parse::<u32>()
                        {
                            if in_rpr_default {
                                table.defaults.size_pt = Some(hp as f32 / 2.0);
                            } else if in_style_rpr && let Some((_, def, _)) = &mut current {
                                def.sz_half_points = Some(hp);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                // Self-closing carriers: <w:b/>, <w:i/>, <w:sz w:val="…"/>.
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "b" | "i" => {
                        let on = !has_attr_false(&e);
                        let key = local.clone();
                        if in_rpr_default {
                            if key == "b" {
                                table.defaults.bold = Some(on);
                            } else {
                                table.defaults.italic = Some(on);
                            }
                        } else if in_style_rpr && let Some((_, def, _)) = &mut current {
                            if key == "b" {
                                def.bold = Some(on);
                            } else {
                                def.italic = Some(on);
                            }
                        }
                    }
                    "sz" => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Ok(hp) = v.trim().parse::<u32>()
                        {
                            if in_rpr_default {
                                table.defaults.size_pt = Some(hp as f32 / 2.0);
                            } else if in_style_rpr && let Some((_, def, _)) = &mut current {
                                def.sz_half_points = Some(hp);
                            }
                        }
                    }
                    "outlineLvl" if current.is_some() => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Ok(lvl) = v.trim().parse::<u8>()
                            && let Some((_, def, _)) = &mut current
                        {
                            def.outline_lvl = Some(lvl);
                        }
                    }
                    "basedOn" if current.is_some() => {
                        if let Some(v) = attr_value(&e, "w:val")
                            && let Some((_, def, _)) = &mut current
                        {
                            def.based_on = Some(v);
                        }
                    }
                    "name" if current.is_some() => {
                        // <w:name w:val="heading 2"/> is usually self-closing
                        // (Event::Empty); the w:val attribute carries it all.
                        if let Some(v) = attr_value(&e, "w:val") {
                            style_name = v;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_name => {
                style_name.push_str(&String::from_utf8_lossy(t.as_bytes()));
            }
            Ok(Event::End(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "name" => in_name = false,
                    "docDefaults" => in_doc_defaults = false,
                    "rPrDefault" => in_rpr_default = false,
                    "rPr" => in_style_rpr = false,
                    "style" => {
                        if let Some((id, mut def, kind)) = current.take() {
                            apply_name_conventions(&mut def, &style_name);
                            if !id.is_empty() {
                                match kind {
                                    StyleKind::Character => {
                                        table.char_styles.insert(id, def);
                                    }
                                    StyleKind::Paragraph => {
                                        table.styles.insert(id, def);
                                    }
                                    StyleKind::Numbering => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    table
}

/// Map a style name to heading conventions (English + Chinese Word).
fn apply_name_conventions(def: &mut StyleDef, raw_name: &str) {
    let name = raw_name.trim().to_lowercase();
    if name.starts_with("heading") || name.starts_with("标题") {
        def.is_heading = true;
        let digits: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(lvl) = digits.parse::<u8>() {
            def.heading_level = Some(lvl);
            def.outline_lvl = Some(lvl.saturating_sub(1));
        }
    } else if name == "title" || name == "标题" {
        def.is_heading = true;
        def.heading_level = Some(1);
    } else if name == "subtitle" || name == "副标题" {
        def.is_heading = true;
        def.heading_level = Some(2);
    }
}

/// Read an attribute value by qualified name.
pub fn attr_value(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| a.value.clone().into_owned())
}

/// Whether a toggle element carries `w:val="0"` / `"false"`.
pub fn has_attr_false(e: &quick_xml::events::BytesStart) -> bool {
    e.attributes().flatten().any(|a| {
        a.key.as_ref() == "w:val" && (a.value.as_ref() == "0" || a.value.as_ref() == "false")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_cascade_inherits_through_based_on() {
        // Base → Mid (italic). The mid style must inherit bold from Base.
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="24"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Base"><w:name w:val="Base"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Mid"><w:name w:val="Mid"/><w:basedOn w:val="Base"/><w:rPr><w:i/></w:rPr></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        let (props, _, _) = table.resolve_paragraph("Mid");
        assert_eq!(props.bold, Some(true), "bold must inherit from Base");
        assert_eq!(props.italic, Some(true), "italic comes from Mid");
        assert_eq!(props.size_pt, Some(12.0), "docDefaults size applies");
    }

    #[test]
    fn derived_style_overrides_base() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:style w:type="paragraph" w:styleId="Base"><w:name w:val="Base"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Leaf"><w:name w:val="Leaf"/><w:basedOn w:val="Base"/><w:rPr><w:b w:val="0"/></w:rPr></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        let (props, _, _) = table.resolve_paragraph("Leaf");
        assert_eq!(
            props.bold,
            Some(false),
            "explicit w:val=0 must clear inherited bold"
        );
    }

    #[test]
    fn style_cycle_does_not_hang() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:style w:type="paragraph" w:styleId="A"><w:name w:val="A"/><w:basedOn w:val="B"/></w:style>
<w:style w:type="paragraph" w:styleId="B"><w:name w:val="B"/><w:basedOn w:val="A"/></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        let (props, _, _) = table.resolve_paragraph("A");
        assert_eq!(props.bold, None, "cycle must not fabricate flags");
    }

    #[test]
    fn heading_names_map_to_levels() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:style w:type="paragraph" w:styleId="H2"><w:name w:val="heading 2"/></w:style>
<w:style w:type="paragraph" w:styleId="T"><w:name w:val="Title"/></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        let (_, outline, heading) = table.resolve_paragraph("H2");
        assert_eq!(heading, Some(2));
        assert_eq!(outline, Some(1));
        let (_, _, title) = table.resolve_paragraph("T");
        assert_eq!(title, Some(1));
    }

    #[test]
    fn doc_defaults_italic_and_numbering_styles_ignored() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:docDefaults><w:rPrDefault><w:rPr><w:i/><w:sz w:val="20"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="numbering" w:styleId="NoList"><w:name w:val="No List"/></w:style>
<w:style w:type="paragraph" w:styleId="P"><w:name w:val="P"/><w:rPr><w:sz w:val="28"/></w:rPr></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        assert_eq!(table.defaults.italic, Some(true));
        assert!(
            table.styles.get("NoList").is_none(),
            "numbering styles must not enter the paragraph table"
        );
        let (props, _, _) = table.resolve_paragraph("P");
        assert_eq!(props.size_pt, Some(14.0), "own sz overrides docDefaults");
        assert_eq!(props.italic, Some(true), "docDefaults italic inherited");
        let (missing, _, _) = table.resolve_paragraph("NoSuchStyle");
        assert_eq!(
            missing.size_pt,
            Some(10.0),
            "unknown id falls back to defaults"
        );
    }

    #[test]
    fn title_subtitle_and_chinese_heading_names() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:style w:type="paragraph" w:styleId="T"><w:name w:val="Title"/></w:style>
<w:style w:type="paragraph" w:styleId="S"><w:name w:val="Subtitle"/></w:style>
<w:style w:type="paragraph" w:styleId="CN"><w:name w:val="标题 1"/></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        let (_, _, title) = table.resolve_paragraph("T");
        assert_eq!(title, Some(1));
        let (_, _, subtitle) = table.resolve_paragraph("S");
        assert_eq!(subtitle, Some(2));
        let (_, outline, cn) = table.resolve_paragraph("CN");
        assert_eq!(cn, Some(1));
        assert_eq!(outline, Some(0));
    }

    #[test]
    fn character_style_inherits_based_on() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:style w:type="character" w:styleId="Base"><w:name w:val="Base"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Derived"><w:name w:val="Derived"/><w:basedOn w:val="Base"/><w:rPr><w:i/></w:rPr></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        let props = table.resolve_character("Derived");
        assert_eq!(props.bold, Some(true));
        assert_eq!(props.italic, Some(true));
        assert_eq!(table.resolve_character("Nope").bold, None);
    }

    #[test]
    fn explicit_off_toggle_parsed_from_empty_element() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:style w:type="paragraph" w:styleId="Off"><w:name w:val="Off"/><w:rPr><w:b w:val="false"/><w:i w:val="0"/></w:rPr></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        let def = table.styles.get("Off").expect("style parsed");
        assert_eq!(def.bold, Some(false));
        assert_eq!(def.italic, Some(false));
    }

    #[test]
    fn start_form_properties_name_text_and_empty_ids() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:docDefaults><w:rPrDefault><w:rPr><w:b>1</w:b><w:sz w:val="18"></w:sz></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="Lvl"><w:name w:val="Custom"/><w:outlineLvl w:val="3"></w:outlineLvl><w:rPr><w:i></w:i><w:sz w:val="30"></w:sz></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="TextName"><w:name>heading 4</w:name></w:style>
<w:style w:type="paragraph" w:styleId="Sub"><w:name w:val="副标题"/></w:style>
<w:style w:type="paragraph" w:styleId="NoDigits"><w:name w:val="Heading"/></w:style>
<w:style w:type="paragraph" styleId="NoAttrs"><w:name w:val="ignored"/></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        // Start-form toggles and sz inside docDefaults and a style body.
        assert_eq!(table.defaults.bold, Some(true));
        assert_eq!(table.defaults.size_pt, Some(9.0));
        let (props, outline, heading) = table.resolve_paragraph("Lvl");
        assert_eq!(props.italic, Some(true));
        assert_eq!(props.size_pt, Some(15.0));
        assert_eq!(outline, Some(3), "outlineLvl from the style body");
        assert_eq!(heading, None, "a custom name is not a heading level");
        // Name carried as element text, not the w:val attribute.
        let (_, _, text_name) = table.resolve_paragraph("TextName");
        assert_eq!(text_name, Some(4));
        // Chinese subtitle convention.
        let (_, _, sub) = table.resolve_paragraph("Sub");
        assert_eq!(sub, Some(2));
        // "Heading" without digits is still a heading: level defaults to 2.
        let (_, _, no_digits) = table.resolve_paragraph("NoDigits");
        assert_eq!(no_digits, Some(2));
        // A style without w:styleId is dropped instead of stored under "".
        assert!(table.styles.get("").is_none());
    }

    #[test]
    fn character_style_resolves() {
        let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="w">
<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style>
</w:styles>"#;
        let table = parse_styles(styles.as_bytes());
        assert_eq!(table.resolve_character("Strong").bold, Some(true));
    }
}
