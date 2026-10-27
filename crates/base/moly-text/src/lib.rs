//! Display-only localization over an immutable source catalogue.
//!
//! The source region selects the simulation and assets. A separate text
//! locale selects official or community text. Neither operation rewrites
//! script instructions, control labels, voice cues or actor identities.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum TextKind {
    Text,
    Speaker,
    Tweet,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLine {
    pub id: String,
    pub kind: TextKind,
    pub step: Option<usize>,
    pub text: String,
    pub source_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceScript {
    key: String,
    lines: Vec<SourceLine>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceDocument {
    schema: String,
    region: String,
    revision: String,
    scripts: Vec<SourceScript>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Translation {
    pub id: String,
    pub source_hash: String,
    pub text: String,
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OverlayDocument {
    schema: String,
    source_region: String,
    source_revision: String,
    locale: String,
    revision: String,
    entries: Vec<Translation>,
}

#[derive(Clone, Debug)]
pub struct SourceCatalog {
    pub region: String,
    pub revision: String,
    lines: HashMap<String, SourceLine>,
    bindings: HashMap<(String, Option<usize>, TextKind), String>,
}

#[derive(Clone, Debug)]
pub struct Overlay {
    pub region: String,
    pub source_revision: String,
    pub locale: String,
    pub revision: String,
    entries: HashMap<String, Translation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOrigin {
    Original,
    Official,
    Community,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayText<'a> {
    pub text: &'a str,
    pub origin: TextOrigin,
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn region(value: &str) -> bool {
    matches!(value, "jp" | "cn" | "tw" | "en" | "kr")
}

fn locale(value: &str) -> bool {
    matches!(value, "ja-JP" | "zh-CN" | "zh-TW" | "en-US" | "ko-KR")
}

fn document<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, String> {
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err("text document exceeds size limit".into());
    }
    serde_json::from_str(text).map_err(|error| format!("invalid text document: {error}"))
}

fn valid_text(text: &str) -> bool {
    text.len() <= MAX_TEXT_BYTES
        && !text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
}

impl SourceCatalog {
    pub fn parse(json: &str) -> Result<Self, String> {
        let doc: SourceDocument = document(json)?;
        if doc.schema != "moly-text-source/1" || !region(&doc.region) || !valid_hash(&doc.revision) {
            return Err("unsupported source text identity".into());
        }
        let mut result = Self {
            region: doc.region,
            revision: doc.revision,
            lines: HashMap::new(),
            bindings: HashMap::new(),
        };
        let mut scripts = HashSet::new();
        for script in doc.scripts {
            if script.key.is_empty() || script.key.len() > 512 || !scripts.insert(script.key.clone()) {
                return Err("empty, duplicate or oversized text script key".into());
            }
            for line in script.lines {
                if line.id.is_empty() || line.id.len() > 160 || !valid_hash(&line.source_hash)
                    || !valid_text(&line.text)
                    || (line.kind == TextKind::Tweet) != line.step.is_none()
                {
                    return Err(format!("invalid source text unit {}", line.id));
                }
                let binding = (script.key.clone(), line.step, line.kind);
                if result.bindings.insert(binding, line.id.clone()).is_some()
                    || result.lines.insert(line.id.clone(), line).is_some()
                {
                    return Err("duplicate source text identity or binding".into());
                }
            }
        }
        Ok(result)
    }

    pub fn line(&self, id: &str) -> Option<&SourceLine> {
        self.lines.get(id)
    }
}

impl Overlay {
    pub fn parse(json: &str) -> Result<Self, String> {
        let doc: OverlayDocument = document(json)?;
        if doc.schema != "moly-text-overlay/1" || !region(&doc.source_region)
            || !locale(&doc.locale) || !valid_hash(&doc.revision) || !valid_hash(&doc.source_revision)
        {
            return Err("unsupported translation identity".into());
        }
        let mut entries = HashMap::new();
        for entry in doc.entries {
            if entry.id.is_empty() || entry.id.len() > 160 || !valid_hash(&entry.source_hash)
                || !valid_text(&entry.text)
                || entry.text.chars().any(|ch| {
                    matches!(ch, '\t' | '\u{ad}' | '\u{fdd0}'..='\u{fdef}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                        || (ch as u32 & 0xffff) >= 0xfffe
                })
                || !matches!(entry.source.as_str(), "human" | "official_cn" | "official_tw" | "official_en" | "official_kr")
            {
                return Err(format!("invalid translation unit {}", entry.id));
            }
            if entries.insert(entry.id.clone(), entry).is_some() {
                return Err("duplicate translation identity".into());
            }
        }
        Ok(Self {
            region: doc.source_region,
            source_revision: doc.source_revision,
            locale: doc.locale,
            revision: doc.revision,
            entries,
        })
    }
}

/// A session pins this value until it ends. Installing another locale or
/// revision for future sessions cannot change an in-progress typewriter.
#[derive(Clone, Debug)]
pub struct TextSession {
    source: Arc<SourceCatalog>,
    pub locale: String,
    official: Option<Arc<Overlay>>,
    community: Option<Arc<Overlay>>,
}

impl TextSession {
    pub fn new(
        source: Arc<SourceCatalog>,
        locale_value: &str,
        official: Option<Arc<Overlay>>,
        community: Option<Arc<Overlay>>,
    ) -> Result<Self, String> {
        if !locale(locale_value) {
            return Err("unsupported text locale".into());
        }
        for overlay in official.iter().chain(community.iter()) {
            if overlay.region != source.region || overlay.locale != locale_value {
                return Err("translation locale or source region does not match".into());
            }
            // Revisions may differ: unchanged stable IDs with equal source
            // hashes remain usable after a source update. Changed lines do not.
        }
        Ok(Self { source, locale: locale_value.into(), official, community })
    }

    pub fn resolve<'a>(
        &'a self,
        script: &str,
        step: Option<usize>,
        kind: TextKind,
        original: &'a str,
    ) -> DisplayText<'a> {
        let fallback = DisplayText { text: original, origin: TextOrigin::Original };
        let Some(id) = self.source.bindings.get(&(script.to_owned(), step, kind)) else {
            return fallback;
        };
        let Some(line) = self.source.lines.get(id).filter(|line| line.text == original) else {
            return fallback;
        };
        let expected_official = match self.locale.as_str() {
            "zh-CN" => "official_cn",
            "zh-TW" => "official_tw",
            "en-US" => "official_en",
            "ko-KR" => "official_kr",
            _ => "",
        };
        for (overlay, origin, source_kind) in [
            (&self.official, TextOrigin::Official, expected_official),
            (&self.community, TextOrigin::Community, "human"),
        ] {
            if let Some(entry) = overlay.as_ref().and_then(|o| o.entries.get(id)) {
                if entry.source_hash == line.source_hash && entry.source == source_kind && !entry.text.trim().is_empty() {
                    return DisplayText { text: &entry.text, origin };
                }
            }
        }
        fallback
    }

    /// Characters needed to render this session's admitted display strings.
    /// The host can prepare glyphs before applying the session to playback.
    pub fn characters(&self) -> Vec<char> {
        // Use precisely the same precedence, source kind, nonempty check and
        // source hash as rendering. A superseded human translation must not
        // require a font when the official line is what the user will see.
        let mut chars: Vec<char> = self.source.bindings.iter().flat_map(|((script, step, kind), id)| {
            let original = &self.source.lines[id].text;
            self.resolve(script, *step, *kind, original).text.chars()
        }).collect();
        chars.sort_unstable();
        chars.dedup();
        chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source(hash: &str) -> Arc<SourceCatalog> {
        Arc::new(SourceCatalog::parse(&json!({
            "schema":"moly-text-source/1", "region":"jp", "revision":"a".repeat(64),
            "scripts":[{"key":"sample", "lines":[{
                "id":"line-a", "kind":"text", "step":2, "text":"Original", "sourceHash":hash
            }]}]
        }).to_string()).unwrap())
    }

    fn overlay(text: &str, provenance: &str, hash: &str) -> Arc<Overlay> {
        Arc::new(Overlay::parse(&json!({
            "schema":"moly-text-overlay/1", "sourceRegion":"jp",
            "sourceRevision":"b".repeat(64), "locale":"zh-CN", "revision":"c".repeat(64),
            "entries":[{"id":"line-a", "sourceHash":hash, "text":text, "source":provenance}]
        }).to_string()).unwrap())
    }

    #[test]
    fn chinese_uses_official_then_community_then_original() {
        let hash = "d".repeat(64);
        let official = overlay("官方", "official_cn", &hash);
        let community = overlay("团队", "human", &hash);
        let session = TextSession::new(source(&hash), "zh-CN", Some(official), Some(community.clone())).unwrap();
        assert_eq!(session.resolve("sample", Some(2), TextKind::Text, "Original"),
                   DisplayText { text: "官方", origin: TextOrigin::Official });
        let session = TextSession::new(source(&hash), "zh-CN", None, Some(community)).unwrap();
        assert_eq!(session.resolve("sample", Some(2), TextKind::Text, "Original").text, "团队");
        let session = TextSession::new(source(&hash), "zh-CN", None, None).unwrap();
        assert_eq!(session.resolve("sample", Some(2), TextKind::Text, "Original").origin, TextOrigin::Original);
    }

    #[test]
    fn source_change_or_wrong_binding_never_applies_stale_text() {
        let old_hash = "d".repeat(64);
        let new_hash = "e".repeat(64);
        let session = TextSession::new(source(&new_hash), "zh-CN", Some(overlay("Old", "official_cn", &old_hash)), None).unwrap();
        assert_eq!(session.resolve("sample", Some(2), TextKind::Text, "Original").origin, TextOrigin::Original);
        assert_eq!(session.resolve("sample", Some(3), TextKind::Text, "Original").origin, TextOrigin::Original);
        assert_eq!(session.resolve("sample", Some(2), TextKind::Text, "Changed").text, "Changed");
    }

    #[test]
    fn locale_switch_does_not_mutate_an_existing_session() {
        let hash = "d".repeat(64);
        let old = TextSession::new(source(&hash), "zh-CN", Some(overlay("中文", "official_cn", &hash)), None).unwrap();
        let new = TextSession::new(source(&hash), "ja-JP", None, None).unwrap();
        assert_eq!(old.resolve("sample", Some(2), TextKind::Text, "Original").text, "中文");
        assert_eq!(new.resolve("sample", Some(2), TextKind::Text, "Original").text, "Original");
    }

    #[test]
    fn overlay_cannot_cross_source_region_or_locale() {
        let hash = "d".repeat(64);
        let zh = overlay("中文", "official_cn", &hash);
        assert!(TextSession::new(source(&hash), "en-US", Some(zh), None).is_err());
        let mut cn = (*source(&hash)).clone();
        cn.region = "cn".into();
        assert!(TextSession::new(Arc::new(cn), "zh-CN", Some(overlay("中文", "official_cn", &hash)), None).is_err());
    }
}
