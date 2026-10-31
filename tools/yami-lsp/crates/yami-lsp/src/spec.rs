//! Editor-facing facts about YLang and YSL.
//!
//! Every limit, group, property and default in this file is taken from the
//! existing parsers and the normative v0.1/v0.2 specifications. Completions and
//! hover documentation are generated from these tables so a feature can never
//! be suggested for a language version that does not support it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Ylang,
    Ysl,
}

impl Language {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ylang => "YLang",
            Self::Ysl => "YSL",
        }
    }

    pub fn from_language_id(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "ylang" => Some(Self::Ylang),
            "ysl" => Some(Self::Ysl),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    V01,
    V02,
}

impl Version {
    pub const ALL: [Self; 2] = [Self::V01, Self::V02];

    pub fn label(self) -> &'static str {
        match self {
            Self::V01 => "0.1",
            Self::V02 => "0.2",
        }
    }

    pub fn ylang_header(self) -> &'static str {
        match self {
            Self::V01 => "@page 0.1",
            Self::V02 => "@page 0.2",
        }
    }

    pub fn ysl_header(self) -> &'static str {
        match self {
            Self::V01 => "@style 0.1",
            Self::V02 => "@style 0.2",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "0.1" => Some(Self::V01),
            "0.2" => Some(Self::V02),
            _ => None,
        }
    }
}

/// Detect a document language from its URI, the client language id and finally
/// a conservative look at the first nonblank line.
pub fn detect_language(uri: &str, language_id: &str, text: &str) -> Option<Language> {
    let lower = uri.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or(&lower);
    if path.ends_with(".ylang") {
        return Some(Language::Ylang);
    }
    if path.ends_with(".ysl") {
        return Some(Language::Ysl);
    }
    if let Some(language) = Language::from_language_id(language_id) {
        return Some(language);
    }
    let header = first_nonblank_line(text)?;
    if header.starts_with("@page") {
        Some(Language::Ylang)
    } else if header.starts_with("@style") {
        Some(Language::Ysl)
    } else {
        None
    }
}

/// The explicitly selected grammar version, if the header is valid.
pub fn detect_version(language: Language, text: &str) -> Option<Version> {
    let header = first_nonblank_line(text)?.trim();
    let rest = match language {
        Language::Ylang => header.strip_prefix("@page")?,
        Language::Ysl => header.strip_prefix("@style")?,
    };
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Version::parse(rest.trim())
}

pub fn version_or_latest(language: Language, text: &str) -> Version {
    detect_version(language, text).unwrap_or(Version::V02)
}

pub fn first_nonblank_line(text: &str) -> Option<&str> {
    text.lines().find(|line| !line.trim().is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Keyword,
    Snippet,
    Property,
    Value,
    Color,
    File,
    EnumMember,
}

impl ItemKind {
    /// LSP `CompletionItemKind` values.
    pub fn lsp(self) -> u8 {
        match self {
            Self::Keyword => 14,
            Self::Snippet => 15,
            Self::Property => 10,
            Self::Value => 12,
            Self::Color => 16,
            Self::File => 17,
            Self::EnumMember => 20,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MetaField {
    pub name: &'static str,
    pub insert: &'static str,
    pub detail: &'static str,
    pub doc: &'static str,
    pub required: bool,
}

pub const META_FIELDS: &[MetaField] = &[
    MetaField {
        name: "title",
        insert: "title ",
        detail: "required metadata field",
        doc: "Page title metadata. Write `title <nonempty text>` indented by exactly two spaces. \
              It is required exactly once.",
        required: true,
    },
    MetaField {
        name: "language",
        insert: "language ",
        detail: "optional metadata field",
        doc: "Optional descriptive language metadata, for example `language en`. \
              Write it indented by exactly two spaces. It may occur at most once and is \
              descriptive text, not a validated language tag.",
        required: false,
    },
];

pub fn meta_field(name: &str) -> Option<&'static MetaField> {
    META_FIELDS.iter().find(|field| field.name == name)
}

pub fn page_doc(version: Version) -> String {
    match version {
        Version::V01 => "### YLang 0.1\n\n\
            The first nonblank line must be exactly `@page 0.1`, followed by metadata \
            indented two spaces (`title` required, `language` optional). Body blocks are \
            headings (`= ` through `====== `), named sections (`:: Name`), YNP links \
            (`> ynp://address/key | Label`), paragraphs and backslash escapes.\n\n\
            Limits: 256 KiB source, 4096 bytes per line, 32 nesting levels. YLang 0.1 has \
            no inline spans, images, lists, separators or local page links."
            .to_string(),
        Version::V02 => "### YLang 0.2\n\n\
            The first nonblank line must be exactly `@page 0.2`, followed by metadata \
            indented two spaces (`title` required, `language` optional). 0.2 adds typed \
            inline spans, semantic containers (`:: card`, `:: row`, `:: group`), lists \
            (`:: list`, `:: steps`), local/native links, images and `---` separators.\n\n\
            Limits: 256 KiB source, 4096 bytes per line, 32 nesting levels, 4096 nodes and \
            16 KiB per paragraph. Rows contain at most four direct child blocks."
            .to_string(),
    }
}

pub fn style_doc(version: Version) -> String {
    match version {
        Version::V01 => "### YSL 0.1\n\n\
            The first nonblank line must be exactly `@style 0.1`. Unindented group names \
            are followed by properties indented two spaces. Styles are cell-oriented and \
            use the original group set: `canvas`, `text`, `heading`, `section`, `link`.\n\n\
            Limits: 16 KiB source. No selectors, expressions, scripts or external resources."
            .to_string(),
        Version::V02 => "### YSL 0.2\n\n\
            The first nonblank line must be exactly `@style 0.2`. The same group/property \
            syntax switches to bounded pixel-based native presentation. 0.2 adds the \
            `card`, `image` and `row` groups plus typography properties.\n\n\
            Limits: 16 KiB source. No absolute positioning, selectors, scripts or external \
            resources."
            .to_string(),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HeaderSpec {
    pub label: &'static str,
    pub insert: &'static str,
    pub detail: &'static str,
    pub doc: &'static str,
    pub version: Version,
}

pub const YLANG_HEADERS: &[HeaderSpec] = &[
    HeaderSpec {
        label: "@page 0.1",
        insert: "@page 0.1\n  title ",
        detail: "YLang 0.1 page header",
        doc: "Selects the original YLang 0.1 grammar and inserts the required `title` \
              metadata line.",
        version: Version::V01,
    },
    HeaderSpec {
        label: "@page 0.2",
        insert: "@page 0.2\n  title ",
        detail: "YLang 0.2 page header",
        doc: "Selects the rich YLang 0.2 grammar and inserts the required `title` metadata \
              line.",
        version: Version::V02,
    },
];

pub const YSL_HEADERS: &[HeaderSpec] = &[
    HeaderSpec {
        label: "@style 0.1",
        insert: "@style 0.1\n",
        detail: "YSL 0.1 stylesheet header",
        doc: "Selects the original cell-based YSL 0.1 grammar.",
        version: Version::V01,
    },
    HeaderSpec {
        label: "@style 0.2",
        insert: "@style 0.2\n",
        detail: "YSL 0.2 stylesheet header",
        doc: "Selects the pixel-based YSL 0.2 grammar, including cards, images and rows.",
        version: Version::V02,
    },
];

#[derive(Clone, Copy, Debug)]
pub struct BlockSpec {
    pub label: &'static str,
    pub insert: &'static str,
    pub detail: &'static str,
    pub doc: &'static str,
    pub version: Version,
    pub kind: ItemKind,
}

pub const DOC_HEADING: &str = "Heading levels 1 through 6 are written as one to six \
    `=` characters followed by a space and nonempty text. Headings are leaves; they cannot \
    contain children. In YLang 0.2 headings support inline spans.";

pub const DOC_SECTION: &str = "A named section starts with `:: Name` and may contain child \
    blocks indented by exactly two more spaces. Section names cannot be empty. Only sections, \
    typed containers and list items may contain children.";

pub const DOC_CARD: &str = "A typed semantic card: `:: card` or `:: card Title`, with an \
    optional plain title. It may contain child blocks. Cards use the shared `card` YSL group; \
    there are no per-node classes or arbitrary styles.";

pub const DOC_ROW: &str = "A typed semantic row: `:: row` or `:: row Title`. Rows contain at \
    most four direct child blocks, use equal columns and stack on compact viewports. Select \
    the gap and minimum column width with the YSL `row` group.";

pub const DOC_GROUP: &str = "A typed semantic group: `:: group` or `:: group Title`. Groups \
    provide vertical flow for child blocks and can be nested within the 32-level document \
    depth limit.";

pub const DOC_LIST: &str = "An unordered list starts with `:: list`; each direct child is an \
    item written `- text`. Items may contain nested blocks. A list requires at least one item.";

pub const DOC_STEPS: &str = "An ordered step list starts with `:: steps`; each direct child is \
    an item written `+ text`, numbered by the renderer. Items may contain nested blocks and a \
    step list requires at least one item.";

pub const DOC_LINK_LOCAL: &str = "A local page link: `> /about | About`. The key names a page \
    on the same node and resolves to `about.ylang` in the site directory. Keys contain only \
    ASCII letters, digits, `_` and `-`; path traversal is rejected.";

pub const DOC_LINK_NATIVE: &str = "A native link to another identity: \
    `> yami://yn1-.../page | Label`. The browser verifies the full identity and transport when \
    the link is activated. The language server never performs network requests.";

pub const DOC_LINK_YNP: &str = "A YLang 0.1 link uses a YNP target: \
    `> ynp://address/key | Label`. The target must start with `ynp://`, contain no whitespace \
    and have a nonempty address and key. The parser does not fetch links. YLang 0.2 also \
    accepts this legacy form.";

pub const DOC_IMAGE: &str = "A same-node image reference: `! asset-key | meaningful \
    alternative text`. Keys are 1..64 ASCII letters, digits, `_` or `-`; no paths, extensions, \
    URLs or remote identities. The image resolves to `assets/<key>.png` and is never fetched \
    automatically by the parser.";

pub const DOC_SEPARATOR: &str = "`---` is a visual separator block. It is available only in \
    YLang 0.2; in 0.1 it is ordinary paragraph text.";

pub const DOC_ESCAPE: &str = "A leading backslash makes the rest of the line literal text \
    without directive or inline interpretation. A bare backslash is invalid.";

pub const DOC_STRONG: &str = "Strong inline span: `[strong: text]`. Text must be nonempty. \
    Inline spans cannot nest; escape literal brackets with `\\[` and `\\]`. Available in \
    YLang 0.2 only.";

pub const DOC_EMPHASIS: &str = "Emphasis inline span: `[em: text]`. Text must be nonempty. \
    Inline spans cannot nest; escape literal brackets with `\\[` and `\\]`. Available in \
    YLang 0.2 only.";

pub const DOC_CODE: &str = "Code inline span: `[code: text]`. Text must be nonempty and is \
    rendered literally. Inline spans cannot nest; escape literal brackets with `\\[` and \
    `\\]`. Available in YLang 0.2 only.";

pub const YLANG_BLOCKS: &[BlockSpec] = &[
    BlockSpec {
        label: "= Heading",
        insert: "= ",
        detail: "heading level 1",
        doc: DOC_HEADING,
        version: Version::V01,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: ":: Section",
        insert: ":: ",
        detail: "named section",
        doc: DOC_SECTION,
        version: Version::V01,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: "> YNP link",
        insert: "> ynp://address/key | Label",
        detail: "YLang 0.1 link",
        doc: DOC_LINK_YNP,
        version: Version::V01,
        kind: ItemKind::Snippet,
    },
    BlockSpec {
        label: "\\ Literal text",
        insert: "\\",
        detail: "escape a reserved prefix",
        doc: DOC_ESCAPE,
        version: Version::V01,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: "= Heading",
        insert: "= ",
        detail: "heading level 1",
        doc: DOC_HEADING,
        version: Version::V02,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: ":: Section",
        insert: ":: ",
        detail: "named section",
        doc: DOC_SECTION,
        version: Version::V02,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: ":: card Title",
        insert: ":: card ",
        detail: "typed card container",
        doc: DOC_CARD,
        version: Version::V02,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: ":: row Title",
        insert: ":: row ",
        detail: "typed row container",
        doc: DOC_ROW,
        version: Version::V02,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: ":: group Title",
        insert: ":: group ",
        detail: "typed group container",
        doc: DOC_GROUP,
        version: Version::V02,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: ":: list",
        insert: ":: list\n  - ",
        detail: "unordered list",
        doc: DOC_LIST,
        version: Version::V02,
        kind: ItemKind::Snippet,
    },
    BlockSpec {
        label: ":: steps",
        insert: ":: steps\n  + ",
        detail: "ordered step list",
        doc: DOC_STEPS,
        version: Version::V02,
        kind: ItemKind::Snippet,
    },
    BlockSpec {
        label: "> Local page link",
        insert: "> /page | Label",
        detail: "same-node page link",
        doc: DOC_LINK_LOCAL,
        version: Version::V02,
        kind: ItemKind::Snippet,
    },
    BlockSpec {
        label: "> Native identity link",
        insert: "> yami://yn1-.../page | Label",
        detail: "link to another identity",
        doc: DOC_LINK_NATIVE,
        version: Version::V02,
        kind: ItemKind::Snippet,
    },
    BlockSpec {
        label: "> YNP link",
        insert: "> ynp://address/key | Label",
        detail: "legacy YNP link",
        doc: DOC_LINK_YNP,
        version: Version::V02,
        kind: ItemKind::Snippet,
    },
    BlockSpec {
        label: "! Image",
        insert: "! asset-key | Alternative text",
        detail: "same-node image",
        doc: DOC_IMAGE,
        version: Version::V02,
        kind: ItemKind::Snippet,
    },
    BlockSpec {
        label: "--- Separator",
        insert: "---",
        detail: "visual separator",
        doc: DOC_SEPARATOR,
        version: Version::V02,
        kind: ItemKind::Keyword,
    },
    BlockSpec {
        label: "\\ Literal text",
        insert: "\\",
        detail: "escape a reserved prefix",
        doc: DOC_ESCAPE,
        version: Version::V02,
        kind: ItemKind::Keyword,
    },
];

#[derive(Clone, Copy, Debug)]
pub struct InlineSpec {
    pub label: &'static str,
    pub insert: &'static str,
    pub marker: &'static str,
    pub doc: &'static str,
}

pub const YLANG_INLINE: &[InlineSpec] = &[
    InlineSpec {
        label: "strong",
        insert: "[strong: ",
        marker: "strong",
        doc: DOC_STRONG,
    },
    InlineSpec {
        label: "em",
        insert: "[em: ",
        marker: "em",
        doc: DOC_EMPHASIS,
    },
    InlineSpec {
        label: "code",
        insert: "[code: ",
        marker: "code",
        doc: DOC_CODE,
    },
];

pub fn blocks_for(version: Version) -> Vec<&'static BlockSpec> {
    YLANG_BLOCKS
        .iter()
        .filter(|block| block.version == version)
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct GroupSpec {
    pub name: &'static str,
    pub detail: &'static str,
    pub doc: &'static str,
    pub version: Version,
}

pub const YSL_GROUPS: &[GroupSpec] = &[
    GroupSpec {
        name: "canvas",
        detail: "page canvas",
        doc: "Canvas group: page width, padding and paper color. In YSL 0.1 sizes are cells; \
              in YSL 0.2 they are logical pixels.",
        version: Version::V01,
    },
    GroupSpec {
        name: "text",
        detail: "body text",
        doc: "Body text group. Sets ink and spacing (and typography in YSL 0.2).",
        version: Version::V01,
    },
    GroupSpec {
        name: "heading",
        detail: "headings",
        doc: "Heading text group. Sets ink and spacing (and typography in YSL 0.2).",
        version: Version::V01,
    },
    GroupSpec {
        name: "section",
        detail: "sections",
        doc: "Section label group. Sets ink and spacing (and typography in YSL 0.2).",
        version: Version::V01,
    },
    GroupSpec {
        name: "link",
        detail: "links",
        doc: "Link group. Sets ink and spacing; YSL 0.2 adds hover and active colors.",
        version: Version::V01,
    },
    GroupSpec {
        name: "canvas",
        detail: "page canvas",
        doc: "Canvas group: pixel width 240..1440, padding 0..64 and paper color. Content \
              always fits the viewport.",
        version: Version::V02,
    },
    GroupSpec {
        name: "text",
        detail: "body text",
        doc: "Body text group with pixel typography: ink, size, weight, alignment, \
              line-height and gap.",
        version: Version::V02,
    },
    GroupSpec {
        name: "heading",
        detail: "headings",
        doc: "Heading text group with pixel typography. Heading levels progressively reduce \
              the configured size, down to a minimum of 12.",
        version: Version::V02,
    },
    GroupSpec {
        name: "section",
        detail: "sections",
        doc: "Section label group with pixel typography.",
        version: Version::V02,
    },
    GroupSpec {
        name: "link",
        detail: "links",
        doc: "Link group with pixel typography plus `hover` and `active` colors applied to \
              the shaped link rectangles.",
        version: Version::V02,
    },
    GroupSpec {
        name: "card",
        detail: "semantic cards",
        doc: "Shared styling for `:: card` containers: paper, ink, border, border width, \
              radius, padding and gap. All cards share one declarative style.",
        version: Version::V02,
    },
    GroupSpec {
        name: "image",
        detail: "same-node images",
        doc: "Same-node image presentation: maximum width and height, alignment and gap. \
              Aspect ratio is preserved.",
        version: Version::V02,
    },
    GroupSpec {
        name: "row",
        detail: "row layout",
        doc: "Row layout: gap between columns and the minimum column width used before a \
              row stacks.",
        version: Version::V02,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Color,
    Integer { min: u32, max: u32 },
    Alignment,
    Weight,
}

#[derive(Clone, Copy, Debug)]
pub struct PropertySpec {
    pub group: &'static str,
    pub name: &'static str,
    pub version: Version,
    pub kind: ValueKind,
    pub default: &'static str,
    pub doc: &'static str,
}

pub const YSL_PROPERTIES: &[PropertySpec] = &[
    // YSL 0.1 — original cell-based presentation.
    PropertySpec {
        group: "canvas",
        name: "width",
        version: Version::V01,
        kind: ValueKind::Integer { min: 20, max: 160 },
        default: "76",
        doc: "Canvas width in terminal cells, 20..160. Default 76.",
    },
    PropertySpec {
        group: "canvas",
        name: "padding",
        version: Version::V01,
        kind: ValueKind::Integer { min: 0, max: 8 },
        default: "2",
        doc: "Canvas padding in cells, 0..8. Default 2.",
    },
    PropertySpec {
        group: "canvas",
        name: "paper",
        version: Version::V01,
        kind: ValueKind::Color,
        default: "#101820",
        doc: "Paper background color as `#RRGGBB`. Default #101820.",
    },
    PropertySpec {
        group: "text",
        name: "ink",
        version: Version::V01,
        kind: ValueKind::Color,
        default: "#d8e2ec",
        doc: "Body text color as `#RRGGBB`. Default #d8e2ec.",
    },
    PropertySpec {
        group: "text",
        name: "gap",
        version: Version::V01,
        kind: ValueKind::Integer { min: 0, max: 3 },
        default: "1",
        doc: "Rows after each text block, 0..3. Default 1.",
    },
    PropertySpec {
        group: "heading",
        name: "ink",
        version: Version::V01,
        kind: ValueKind::Color,
        default: "#61dafb",
        doc: "Heading color as `#RRGGBB`. Default #61dafb.",
    },
    PropertySpec {
        group: "heading",
        name: "gap",
        version: Version::V01,
        kind: ValueKind::Integer { min: 0, max: 3 },
        default: "1",
        doc: "Rows after headings, 0..3. Default 1.",
    },
    PropertySpec {
        group: "section",
        name: "ink",
        version: Version::V01,
        kind: ValueKind::Color,
        default: "#b4d273",
        doc: "Section label color as `#RRGGBB`. Default #b4d273.",
    },
    PropertySpec {
        group: "section",
        name: "gap",
        version: Version::V01,
        kind: ValueKind::Integer { min: 0, max: 3 },
        default: "0",
        doc: "Rows after sections, 0..3. Default 0.",
    },
    PropertySpec {
        group: "link",
        name: "ink",
        version: Version::V01,
        kind: ValueKind::Color,
        default: "#ffa970",
        doc: "Link color as `#RRGGBB`. Default #ffa970.",
    },
    PropertySpec {
        group: "link",
        name: "gap",
        version: Version::V01,
        kind: ValueKind::Integer { min: 0, max: 3 },
        default: "1",
        doc: "Rows after links, 0..3. Default 1.",
    },
    // YSL 0.2 — pixel-based native presentation.
    PropertySpec {
        group: "canvas",
        name: "width",
        version: Version::V02,
        kind: ValueKind::Integer {
            min: 240,
            max: 1440,
        },
        default: "960",
        doc: "Canvas width in logical pixels, 240..1440. Default 960. Content always fits \
              the viewport.",
    },
    PropertySpec {
        group: "canvas",
        name: "padding",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 64 },
        default: "24",
        doc: "Canvas padding in logical pixels, 0..64. Default 24.",
    },
    PropertySpec {
        group: "canvas",
        name: "paper",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#101820",
        doc: "Paper background color as `#RRGGBB`. Default #101820.",
    },
    PropertySpec {
        group: "text",
        name: "ink",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#d8e2ec",
        doc: "Body text color as `#RRGGBB`. Default #d8e2ec.",
    },
    PropertySpec {
        group: "text",
        name: "size",
        version: Version::V02,
        kind: ValueKind::Integer { min: 12, max: 72 },
        default: "18",
        doc: "Font size in logical pixels, 12..72. Default 18.",
    },
    PropertySpec {
        group: "text",
        name: "weight",
        version: Version::V02,
        kind: ValueKind::Weight,
        default: "400",
        doc: "Font weight: 400 (regular) or 600 (semibold). Default 400.",
    },
    PropertySpec {
        group: "text",
        name: "align",
        version: Version::V02,
        kind: ValueKind::Alignment,
        default: "left",
        doc: "Text alignment: `left`, `center` or `right`. Default left.",
    },
    PropertySpec {
        group: "text",
        name: "line-height",
        version: Version::V02,
        kind: ValueKind::Integer { min: 100, max: 220 },
        default: "150",
        doc: "Line height as a percentage of font size, 100..220. Default 150.",
    },
    PropertySpec {
        group: "text",
        name: "gap",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 64 },
        default: "12",
        doc: "Gap after text blocks in logical pixels, 0..64. Default 12.",
    },
    PropertySpec {
        group: "heading",
        name: "ink",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#61dafb",
        doc: "Heading color as `#RRGGBB`. Default #61dafb.",
    },
    PropertySpec {
        group: "heading",
        name: "size",
        version: Version::V02,
        kind: ValueKind::Integer { min: 12, max: 72 },
        default: "36",
        doc: "Heading font size in logical pixels, 12..72. Default 36. Heading levels \
              progressively reduce it, with a minimum of 12.",
    },
    PropertySpec {
        group: "heading",
        name: "weight",
        version: Version::V02,
        kind: ValueKind::Weight,
        default: "600",
        doc: "Heading font weight: 400 or 600. Default 600.",
    },
    PropertySpec {
        group: "heading",
        name: "align",
        version: Version::V02,
        kind: ValueKind::Alignment,
        default: "left",
        doc: "Heading alignment: `left`, `center` or `right`. Default left.",
    },
    PropertySpec {
        group: "heading",
        name: "line-height",
        version: Version::V02,
        kind: ValueKind::Integer { min: 100, max: 220 },
        default: "150",
        doc: "Heading line height as a percentage of font size, 100..220. Default 150.",
    },
    PropertySpec {
        group: "heading",
        name: "gap",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 64 },
        default: "16",
        doc: "Gap after headings in logical pixels, 0..64. Default 16.",
    },
    PropertySpec {
        group: "section",
        name: "ink",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#b4d273",
        doc: "Section label color as `#RRGGBB`. Default #b4d273.",
    },
    PropertySpec {
        group: "section",
        name: "size",
        version: Version::V02,
        kind: ValueKind::Integer { min: 12, max: 72 },
        default: "23",
        doc: "Section label font size in logical pixels, 12..72. Default 23.",
    },
    PropertySpec {
        group: "section",
        name: "weight",
        version: Version::V02,
        kind: ValueKind::Weight,
        default: "600",
        doc: "Section label weight: 400 or 600. Default 600.",
    },
    PropertySpec {
        group: "section",
        name: "align",
        version: Version::V02,
        kind: ValueKind::Alignment,
        default: "left",
        doc: "Section label alignment: `left`, `center` or `right`. Default left.",
    },
    PropertySpec {
        group: "section",
        name: "line-height",
        version: Version::V02,
        kind: ValueKind::Integer { min: 100, max: 220 },
        default: "150",
        doc: "Section line height as a percentage of font size, 100..220. Default 150.",
    },
    PropertySpec {
        group: "section",
        name: "gap",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 64 },
        default: "12",
        doc: "Gap after sections in logical pixels, 0..64. Default 12.",
    },
    PropertySpec {
        group: "link",
        name: "ink",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#ffa970",
        doc: "Link color as `#RRGGBB`. Default #ffa970.",
    },
    PropertySpec {
        group: "link",
        name: "size",
        version: Version::V02,
        kind: ValueKind::Integer { min: 12, max: 72 },
        default: "18",
        doc: "Link font size in logical pixels, 12..72. Default 18.",
    },
    PropertySpec {
        group: "link",
        name: "weight",
        version: Version::V02,
        kind: ValueKind::Weight,
        default: "600",
        doc: "Link font weight: 400 or 600. Default 600.",
    },
    PropertySpec {
        group: "link",
        name: "align",
        version: Version::V02,
        kind: ValueKind::Alignment,
        default: "left",
        doc: "Link alignment: `left`, `center` or `right`. Default left.",
    },
    PropertySpec {
        group: "link",
        name: "line-height",
        version: Version::V02,
        kind: ValueKind::Integer { min: 100, max: 220 },
        default: "150",
        doc: "Link line height as a percentage of font size, 100..220. Default 150.",
    },
    PropertySpec {
        group: "link",
        name: "gap",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 64 },
        default: "10",
        doc: "Gap after links in logical pixels, 0..64. Default 10.",
    },
    PropertySpec {
        group: "link",
        name: "hover",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#ffd5a0",
        doc: "Link hover color as `#RRGGBB`. Default #ffd5a0. Applied to the actual shaped \
              link rectangles.",
    },
    PropertySpec {
        group: "link",
        name: "active",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#61dafb",
        doc: "Link active color as `#RRGGBB`. Default #61dafb.",
    },
    PropertySpec {
        group: "card",
        name: "paper",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#192531",
        doc: "Card background color as `#RRGGBB`. Default #192531.",
    },
    PropertySpec {
        group: "card",
        name: "ink",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#d8e2ec",
        doc: "Card text color as `#RRGGBB`. Default #d8e2ec.",
    },
    PropertySpec {
        group: "card",
        name: "border",
        version: Version::V02,
        kind: ValueKind::Color,
        default: "#405970",
        doc: "Card border color as `#RRGGBB`. Default #405970.",
    },
    PropertySpec {
        group: "card",
        name: "border-width",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 4 },
        default: "1",
        doc: "Card border width in logical pixels, 0..4. Default 1.",
    },
    PropertySpec {
        group: "card",
        name: "radius",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 32 },
        default: "14",
        doc: "Card corner radius in logical pixels, 0..32. Default 14.",
    },
    PropertySpec {
        group: "card",
        name: "padding",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 48 },
        default: "20",
        doc: "Card padding in logical pixels, 0..48. Default 20.",
    },
    PropertySpec {
        group: "card",
        name: "gap",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 64 },
        default: "20",
        doc: "Gap between card children in logical pixels, 0..64. Default 20.",
    },
    PropertySpec {
        group: "row",
        name: "gap",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 64 },
        default: "20",
        doc: "Gap between row columns in logical pixels, 0..64. Default 20.",
    },
    PropertySpec {
        group: "row",
        name: "min-width",
        version: Version::V02,
        kind: ValueKind::Integer { min: 160, max: 640 },
        default: "240",
        doc: "Minimum row column width in logical pixels, 160..640. Default 240. The row \
              stacks when a column cannot meet this width.",
    },
    PropertySpec {
        group: "image",
        name: "width",
        version: Version::V02,
        kind: ValueKind::Integer { min: 16, max: 1440 },
        default: "800",
        doc: "Maximum image width in logical pixels, 16..1440. Default 800. Aspect ratio is \
              preserved.",
    },
    PropertySpec {
        group: "image",
        name: "height",
        version: Version::V02,
        kind: ValueKind::Integer { min: 16, max: 1536 },
        default: "1024",
        doc: "Maximum image height in logical pixels, 16..1536. Default 1024.",
    },
    PropertySpec {
        group: "image",
        name: "align",
        version: Version::V02,
        kind: ValueKind::Alignment,
        default: "center",
        doc: "Image alignment: `left`, `center` or `right`. Default center.",
    },
    PropertySpec {
        group: "image",
        name: "gap",
        version: Version::V02,
        kind: ValueKind::Integer { min: 0, max: 64 },
        default: "16",
        doc: "Gap after images in logical pixels, 0..64. Default 16.",
    },
];

pub fn groups_for(version: Version) -> Vec<&'static GroupSpec> {
    YSL_GROUPS
        .iter()
        .filter(|group| group.version == version)
        .collect()
}

pub fn group_spec(name: &str, version: Version) -> Option<&'static GroupSpec> {
    YSL_GROUPS
        .iter()
        .find(|group| group.version == version && group.name == name)
}

pub fn properties_for(group: &str, version: Version) -> Vec<&'static PropertySpec> {
    YSL_PROPERTIES
        .iter()
        .filter(|property| property.version == version && property.group == group)
        .collect()
}

pub fn property_spec(group: &str, name: &str, version: Version) -> Option<&'static PropertySpec> {
    YSL_PROPERTIES.iter().find(|property| {
        property.version == version && property.group == group && property.name == name
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueSuggestion {
    pub label: String,
    pub insert: String,
    pub detail: String,
    pub doc: String,
    pub kind: ItemKind,
}

pub fn value_suggestions(property: &PropertySpec) -> Vec<ValueSuggestion> {
    let mut values = Vec::new();
    match property.kind {
        ValueKind::Alignment => {
            for value in ["left", "center", "right"] {
                values.push(ValueSuggestion {
                    label: value.to_string(),
                    insert: value.to_string(),
                    detail: "alignment".to_string(),
                    doc: format!(
                        "`{value}` is a supported {} alignment. {}",
                        property.name, property.doc
                    ),
                    kind: ItemKind::EnumMember,
                });
            }
        }
        ValueKind::Weight => {
            for (value, description) in [("400", "regular"), ("600", "semibold")] {
                values.push(ValueSuggestion {
                    label: value.to_string(),
                    insert: value.to_string(),
                    detail: "font weight".to_string(),
                    doc: format!("Font weight {value} ({description}). {}", property.doc),
                    kind: ItemKind::EnumMember,
                });
            }
        }
        ValueKind::Color => {
            for value in [
                property.default,
                "#101820",
                "#d8e2ec",
                "#61dafb",
                "#b4d273",
                "#ffa970",
                "#192531",
                "#405970",
            ] {
                if !values
                    .iter()
                    .any(|existing: &ValueSuggestion| existing.label == value)
                {
                    values.push(ValueSuggestion {
                        label: value.to_string(),
                        insert: value.to_string(),
                        detail: "color #RRGGBB".to_string(),
                        doc: format!("Valid `#RRGGBB` color value. {}", property.doc),
                        kind: ItemKind::Color,
                    });
                }
            }
        }
        ValueKind::Integer { min, max } => {
            values.push(ValueSuggestion {
                label: property.default.to_string(),
                insert: property.default.to_string(),
                detail: format!("integer {min}..{max}"),
                doc: format!("{}. Supported range: {min}..{max}.", property.doc),
                kind: ItemKind::Value,
            });
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_detection_is_strict() {
        assert_eq!(
            detect_version(Language::Ylang, "@page 0.1\n  title A"),
            Some(Version::V01)
        );
        assert_eq!(
            detect_version(Language::Ylang, "\r\n@page 0.2\r\n  title A"),
            Some(Version::V02)
        );
        assert_eq!(
            detect_version(Language::Ylang, "@page 0.3\n  title A"),
            None
        );
        assert_eq!(
            detect_version(Language::Ysl, "@style 0.2"),
            Some(Version::V02)
        );
        assert_eq!(
            detect_version(Language::Ysl, "@style 0.1"),
            Some(Version::V01)
        );
        assert_eq!(detect_version(Language::Ysl, "@style2"), None);
    }

    #[test]
    fn ysl_tables_do_not_leak_02_features_into_01() {
        let v01: Vec<_> = groups_for(Version::V01)
            .iter()
            .map(|group| group.name)
            .collect();
        assert_eq!(v01, ["canvas", "text", "heading", "section", "link"]);
        assert!(group_spec("card", Version::V01).is_none());
        assert!(property_spec("text", "size", Version::V01).is_none());
        assert!(property_spec("text", "size", Version::V02).is_some());
        assert_eq!(
            property_spec("canvas", "width", Version::V01).unwrap().kind,
            ValueKind::Integer { min: 20, max: 160 }
        );
        assert_eq!(
            property_spec("canvas", "width", Version::V02).unwrap().kind,
            ValueKind::Integer {
                min: 240,
                max: 1440
            }
        );
        assert!(property_spec("card", "radius", Version::V02).is_some());
    }

    #[test]
    fn language_detection_prefers_extension_then_language_id_then_header() {
        assert_eq!(
            detect_language("file:///a/b.YLANG", "plaintext", ""),
            Some(Language::Ylang)
        );
        assert_eq!(
            detect_language("untitled:1", "ysl", ""),
            Some(Language::Ysl)
        );
        assert_eq!(
            detect_language("untitled:1", "", "@page 0.2"),
            Some(Language::Ylang)
        );
        assert_eq!(detect_language("untitled:1", "", ""), None);
    }
}
