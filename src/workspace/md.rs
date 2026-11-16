//! A small GitHub-flavoured Markdown renderer for READMEs, PR threads and release
//! notes. It draws inline code in the monospace font (GPUI highlights can't switch
//! fonts, so text is built from runs) and spaces blocks itself.
use super::ui::{col, row};
use crate::{
    avatars::{Avatars, Source},
    theme::Palette,
};
use gpui::prelude::FluentBuilder;
use gpui::*;
use markdown::mdast::{AlignKind, Node};
use std::{cell::Cell, ops::Range};

/// Where a README sits, so relative image paths resolve inside its repository.
#[derive(Clone)]
pub struct Base {
    pub slug: String,
    pub reference: String,
    /// The README's folder, without slashes at either end ("" for the root).
    pub dir: String,
}

/// How source newlines inside a paragraph read: Markdown files treat them as
/// spaces; GitHub comments and release notes treat them as line breaks.
#[derive(Clone, Copy, PartialEq)]
pub enum Breaks {
    Soft,
    Hard,
}

struct Ctx {
    id: SharedString,
    next: Cell<usize>,
    colors: Palette,
    mono: SharedString,
    mono_size: Pixels,
    breaks: Breaks,
    images: Option<Avatars>,
    base: Option<Base>,
}
impl Ctx {
    fn id(&self, what: &str) -> ElementId {
        let n = self.next.get();
        self.next.set(n + 1);
        ElementId::Name(format!("{}/{what}{n}", self.id).into())
    }
}

#[derive(Clone, Default)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

/// One paragraph's worth of styled text, with link ranges for click handling.
#[derive(Default)]
struct Inline {
    text: String,
    runs: Vec<(usize, Style)>,
    links: Vec<(Range<usize>, String)>,
}
impl Inline {
    fn push(&mut self, s: &str, style: &Style) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        self.runs.push((s.len(), style.clone()));
        if let Some(url) = &style.link {
            match self.links.last_mut() {
                Some((r, u)) if u == url && r.end == start => r.end = self.text.len(),
                _ => self.links.push((start..self.text.len(), url.clone())),
            }
        }
    }
    fn walk(&mut self, nodes: &[Node], style: &Style, breaks: Breaks) {
        for n in nodes {
            match n {
                Node::Text(t) => match breaks {
                    Breaks::Soft => self.push(&t.value.replace('\n', " "), style),
                    Breaks::Hard => self.push(&t.value, style),
                },
                Node::Break(_) => self.push("\n", style),
                Node::InlineCode(c) => self.push(
                    &c.value,
                    &Style {
                        code: true,
                        ..style.clone()
                    },
                ),
                Node::Strong(s) => self.walk(
                    &s.children,
                    &Style {
                        bold: true,
                        ..style.clone()
                    },
                    breaks,
                ),
                Node::Emphasis(e) => self.walk(
                    &e.children,
                    &Style {
                        italic: true,
                        ..style.clone()
                    },
                    breaks,
                ),
                Node::Delete(d) => self.walk(
                    &d.children,
                    &Style {
                        strike: true,
                        ..style.clone()
                    },
                    breaks,
                ),
                Node::Link(l) => self.walk(
                    &l.children,
                    &Style {
                        link: Some(l.url.clone()),
                        ..style.clone()
                    },
                    breaks,
                ),
                Node::LinkReference(l) => self.walk(&l.children, style, breaks),
                Node::Image(i) => self.push(
                    &format!("[{}]", if i.alt.is_empty() { "image" } else { &i.alt }),
                    &Style {
                        link: Some(i.url.clone()),
                        ..style.clone()
                    },
                ),
                Node::Html(h) => {
                    let lower = h.value.to_ascii_lowercase();
                    if lower.starts_with("<br") {
                        self.push("\n", style);
                    }
                }
                other => {
                    if let Some(children) = other.children() {
                        self.walk(children, style, breaks);
                    }
                }
            }
        }
    }
    fn element(self, cx: &Ctx) -> AnyElement {
        let colors = cx.colors;
        let runs = self
            .runs
            .iter()
            .map(|(len, s)| TextRun {
                len: *len,
                font: Font {
                    family: if s.code { cx.mono.clone() } else { ".SystemUIFont".into() },
                    features: FontFeatures::default(),
                    fallbacks: None,
                    weight: if s.bold { FontWeight::SEMIBOLD } else { FontWeight::NORMAL },
                    style: if s.italic { FontStyle::Italic } else { FontStyle::Normal },
                },
                color: rgb(if s.link.is_some() {
                    colors.blue
                } else {
                    colors.text
                })
                .into(),
                background_color: s.code.then(|| rgb(colors.hover).into()),
                underline: s.link.as_ref().map(|_| UnderlineStyle {
                    thickness: px(1.),
                    color: Some(Hsla::from(rgb(colors.blue)).opacity(0.4)),
                    wavy: false,
                }),
                strikethrough: s.strike.then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                }),
            })
            .collect::<Vec<_>>();
        let styled = StyledText::new(self.text).with_runs(runs);
        if self.links.is_empty() {
            return styled.into_any_element();
        }
        let (ranges, urls): (Vec<_>, Vec<_>) = self.links.into_iter().unzip();
        InteractiveText::new(cx.id("text"), styled)
            .on_click(ranges, move |ix, _, cx| {
                if let Some(url) = urls.get(ix)
                    && (url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:"))
                {
                    cx.open_url(url);
                }
            })
            .into_any_element()
    }
}

/// Whether a fence's info string marks a GitHub suggested change.
fn is_suggestion(lang: Option<&str>) -> bool {
    lang.is_some_and(|l| l.split(|c: char| c == ':' || c.is_whitespace()).next() == Some("suggestion"))
}

fn inline(nodes: &[Node], cx: &Ctx) -> AnyElement {
    let mut i = Inline::default();
    i.walk(nodes, &Style::default(), cx.breaks);
    i.element(cx)
}

/// Text inside HTML blocks (e.g. `<details>`), without the tags.
fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            c if !tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An image in Markdown or HTML.
#[derive(Debug, PartialEq)]
struct Img {
    url: String,
    alt: String,
    link: Option<String>,
    width: Option<f32>,
}
/// A paragraph that holds images, split into text and images; None when it
/// has no images (the common case, drawn as one run of text).
enum Piece {
    Text(Vec<Node>),
    Image(Img),
}
fn pieces(nodes: &[Node]) -> Option<Vec<Piece>> {
    let image_link = |n: &Node| match n {
        Node::Link(l) => {
            l.children.iter().any(|c| matches!(c, Node::Image(_)))
                && l.children.iter().all(|c| matches!(c, Node::Image(_)) || matches!(c, Node::Text(t) if t.value.trim().is_empty()))
        }
        _ => false,
    };
    let has = nodes.iter().any(|n| {
        matches!(n, Node::Image(_)) || image_link(n) || matches!(n, Node::Html(h) if h.value.to_ascii_lowercase().contains("<img"))
    });
    if !has {
        return None;
    }
    let mut out = vec![];
    let mut text: Vec<Node> = vec![];
    let flush = |text: &mut Vec<Node>, out: &mut Vec<Piece>| {
        let blank = text.iter().all(|n| matches!(n, Node::Text(t) if t.value.trim().is_empty()) || matches!(n, Node::Break(_)));
        if !blank {
            out.push(Piece::Text(std::mem::take(text)));
        }
        text.clear();
    };
    for n in nodes {
        match n {
            Node::Image(i) => {
                flush(&mut text, &mut out);
                out.push(Piece::Image(Img {
                    url: i.url.clone(),
                    alt: i.alt.clone(),
                    link: None,
                    width: None,
                }));
            }
            Node::Link(l) if image_link(n) => {
                flush(&mut text, &mut out);
                for c in &l.children {
                    if let Node::Image(i) = c {
                        out.push(Piece::Image(Img {
                            url: i.url.clone(),
                            alt: i.alt.clone(),
                            link: Some(l.url.clone()),
                            width: None,
                        }));
                    }
                }
            }
            Node::Html(h) if h.value.to_ascii_lowercase().contains("<img") => {
                flush(&mut text, &mut out);
                out.extend(html_images(&h.value).into_iter().map(Piece::Image));
            }
            other => text.push(other.clone()),
        }
    }
    flush(&mut text, &mut out);
    Some(out)
}
/// `<img>` tags in raw HTML: source, alt text and width.
fn html_images(html: &str) -> Vec<Img> {
    let lower = html.to_ascii_lowercase();
    let mut out = vec![];
    let mut from = 0;
    while let Some(at) = lower[from..].find("<img") {
        let start = from + at;
        let end = lower[start..].find('>').map(|e| start + e).unwrap_or(lower.len());
        let tag = &html[start..end];
        let attr = |name: &str| -> Option<String> {
            let tl = tag.to_ascii_lowercase();
            let mut search = 0;
            while let Some(i) = tl[search..].find(name) {
                let i = search + i;
                search = i + name.len();
                // A whole attribute name, followed by `=`.
                let before = tl[..i].chars().last().unwrap_or(' ');
                let after = tl[i + name.len()..].trim_start();
                if !before.is_whitespace() || !after.starts_with('=') {
                    continue;
                }
                let value = after[1..].trim_start();
                let offset = tag.len() - value.len();
                let raw = &tag[offset..];
                return Some(match raw.chars().next() {
                    Some(q @ ('"' | '\'')) => raw[1..].split(q).next().unwrap_or("").to_string(),
                    _ => raw.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("").to_string(),
                });
            }
            None
        };
        if let Some(url) = attr("src").filter(|u| !u.is_empty()) {
            out.push(Img {
                url,
                alt: attr("alt").unwrap_or_default(),
                link: None,
                width: attr("width").and_then(|w| w.trim_end_matches("px").parse().ok()),
            });
        }
        from = end;
    }
    out
}
/// Resolve an image URL: absolute URLs as they are, relative paths inside the
/// README's repository.
fn resolve(url: &str, base: Option<&Base>) -> Option<Source> {
    let url = url.trim();
    if url.starts_with("https://") || url.starts_with("http://") {
        return Some(Source::Url(url.to_string()));
    }
    if let Some(rest) = url.strip_prefix("//") {
        return Some(Source::Url(format!("https://{rest}")));
    }
    if url.contains(':') || url.starts_with('#') {
        return None;
    }
    let base = base?;
    let clean = url.split(['?', '#']).next().unwrap_or("");
    let mut parts: Vec<&str> = if clean.starts_with('/') {
        vec![]
    } else {
        base.dir.split('/').filter(|p| !p.is_empty()).collect()
    };
    for p in clean.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    Some(Source::Repo {
        slug: base.slug.clone(),
        reference: base.reference.clone(),
        path: parts.join("/"),
    })
}
fn image(i: Img, cx: &Ctx) -> AnyElement {
    let colors = cx.colors;
    let src = resolve(&i.url, cx.base.as_ref());
    let loaded = match (&cx.images, &src) {
        (Some(images), Some(src)) => images.image(src),
        _ => Err(()),
    };
    let alt = if i.alt.is_empty() { "image".to_string() } else { i.alt.clone() };
    let el = match loaded {
        Ok(Some(data)) => img(data)
            .max_w_full()
            .when_some(i.width, |m, w| m.w(px(w)))
            .into_any_element(),
        Ok(None) => div()
            .px(px(6.))
            .py(px(2.))
            .rounded(px(4.))
            .bg(rgb(colors.hover))
            .text_size(px(12.))
            .text_color(rgb(colors.quiet))
            .child(alt)
            .into_any_element(),
        Err(()) => div().text_size(px(12.)).text_color(rgb(colors.muted)).child(format!("[{alt}]")).into_any_element(),
    };
    // Linked images open their link; others open the image itself.
    let target = i.link.or(match &src {
        Some(Source::Url(u)) => Some(u.clone()),
        _ => None,
    });
    match target.filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
        Some(url) => div()
            .id(cx.id("img"))
            .max_w_full()
            .cursor_pointer()
            .child(el)
            .on_click(move |_, _, cx| cx.open_url(&url))
            .into_any_element(),
        None => div().max_w_full().child(el).into_any_element(),
    }
}
/// Text and images in reading order; images wrap like inline content (badge
/// rows) and are never wider than the column.
fn render_pieces(parts: Vec<Piece>, cx: &Ctx) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .line_height(relative(1.55))
        .children(parts.into_iter().map(|p| match p {
            Piece::Text(nodes) => div().max_w_full().child(inline(&nodes, cx)).into_any_element(),
            Piece::Image(i) => image(i, cx),
        }))
        .into_any_element()
}

fn blocks(nodes: &[Node], cx: &Ctx) -> Vec<AnyElement> {
    nodes.iter().filter_map(|n| block(n, cx)).collect()
}

fn block(node: &Node, cx: &Ctx) -> Option<AnyElement> {
    let colors = cx.colors;
    Some(match node {
        Node::Paragraph(p) => match pieces(&p.children) {
            Some(parts) => render_pieces(parts, cx),
            None => div().w_full().line_height(relative(1.55)).child(inline(&p.children, cx)).into_any_element(),
        },
        Node::Heading(h) => {
            let size = match h.depth {
                1 => 20.,
                2 => 17.,
                3 => 15.,
                _ => 14.,
            };
            div()
                .w_full()
                .pt(px(if h.depth <= 2 { 6. } else { 2. }))
                .text_size(px(size))
                .font_weight(FontWeight::SEMIBOLD)
                .when(h.depth <= 2, |d| d.pb(px(6.)).border_b_1().border_color(rgb(colors.row_border)))
                .child(inline(&h.children, cx))
                .into_any_element()
        }
        Node::List(l) => {
            let start = l.start.unwrap_or(1);
            let mut list = col().w_full().gap(px(if l.spread { 8. } else { 3. }));
            for (i, item) in l.children.iter().enumerate() {
                let Node::ListItem(item) = item else { continue };
                let marker = match item.checked {
                    Some(done) => div()
                        .mt(px(3.))
                        .size(px(13.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(rgb(if done { colors.green } else { colors.border }))
                        .when(done, |d| d.bg(rgb(colors.green)))
                        .into_any_element(),
                    None if l.ordered => div()
                        .text_color(rgb(colors.muted))
                        .child(format!("{}.", start as usize + i))
                        .into_any_element(),
                    None => div().text_color(rgb(colors.muted)).child("•").into_any_element(),
                };
                list = list.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(div().w(px(18.)).flex_shrink_0().flex().justify_end().line_height(relative(1.55)).child(marker))
                        .child(col().flex_1().min_w_0().gap(px(6.)).children(blocks(&item.children, cx))),
                );
            }
            list.into_any_element()
        }
        Node::Code(c) => {
            let code = c.value.clone();
            let suggestion = is_suggestion(c.lang.as_deref());
            let code_box = div()
                .relative()
                .w_full()
                .group("code")
                .child(
                    div()
                        .id(cx.id("code"))
                        .w_full()
                        .overflow_x_scroll()
                        // A sideways swipe scrolls the code; the page keeps vertical ones.
                        .map(|mut d| {
                            d.style().restrict_scroll_to_axis = Some(true);
                            d
                        })
                        .p(px(10.))
                        .rounded(px(6.))
                        .bg(rgb(colors.panel))
                        .border_1()
                        .border_color(rgb(colors.row_border))
                        .font_family(cx.mono.clone())
                        .text_size(cx.mono_size)
                        .line_height(relative(1.5))
                        .children(c.value.lines().map(|l| div().whitespace_nowrap().min_h(px(4.)).child(l.replace('\t', "    ")))),
                )
                .child(
                    div()
                        .id(cx.id("copy"))
                        .absolute()
                        .top(px(6.))
                        .right(px(6.))
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .text_size(px(11.))
                        .text_color(rgb(colors.muted))
                        .bg(rgb(colors.background))
                        .border_1()
                        .border_color(rgb(colors.border))
                        .cursor_pointer()
                        .opacity(0.)
                        .group_hover("code", |d| d.opacity(1.))
                        .child("Copy")
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(code.clone()))),
                );
            // A review suggestion is the replacement for the lines commented on.
            if suggestion {
                col()
                    .w_full()
                    .gap(px(4.))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(rgb(colors.green)).child("Suggested change"))
                    .child(code_box)
                    .into_any_element()
            } else {
                code_box.into_any_element()
            }
        }
        Node::Blockquote(b) => col()
            .w_full()
            .gap(px(8.))
            .pl_3()
            .border_l_2()
            .border_color(rgb(colors.border))
            .text_color(rgb(colors.muted))
            .children(blocks(&b.children, cx))
            .into_any_element(),
        Node::ThematicBreak(_) => div().w_full().h(px(1.)).bg(rgb(colors.border)).into_any_element(),
        Node::Table(t) => {
            let mut table = col()
                .w_full()
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(colors.border))
                .overflow_hidden();
            for (r, tr) in t.children.iter().enumerate() {
                let Node::TableRow(tr) = tr else { continue };
                let mut line = row()
                    .w_full()
                    .when(r > 0, |d| d.border_t_1().border_color(rgb(colors.row_border)))
                    .when(r == 0, |d| d.bg(rgb(colors.chrome)).font_weight(FontWeight::SEMIBOLD));
                for (c, cell) in tr.children.iter().enumerate() {
                    let Node::TableCell(cell) = cell else { continue };
                    let align = t.align.get(c).copied().unwrap_or(AlignKind::None);
                    line = line.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py(px(5.))
                            .when(c > 0, |d| d.border_l_1().border_color(rgb(colors.row_border)))
                            .flex()
                            .map(|d| match align {
                                AlignKind::Center => d.justify_center(),
                                AlignKind::Right => d.justify_end(),
                                _ => d,
                            })
                            .child(inline(&cell.children, cx)),
                    );
                }
                table = table.child(line);
            }
            table.into_any_element()
        }
        Node::Html(h) => {
            let images = html_images(&h.value);
            let text = strip_tags(&h.value);
            if !images.is_empty() {
                // `<p align="center"><img …></p>` and pasted screenshots.
                let centered = h.value.to_ascii_lowercase().contains("align=\"center\"");
                return Some(
                    div()
                        .w_full()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .when(centered, |d| d.justify_center())
                        .children(images.into_iter().map(|i| image(i, cx)))
                        .when(!text.is_empty(), |d| d.child(div().w_full().text_color(rgb(colors.muted)).child(text)))
                        .into_any_element(),
                );
            }
            if text.is_empty() {
                return None;
            }
            div().w_full().text_color(rgb(colors.muted)).child(text).into_any_element()
        }
        Node::Root(r) => col().w_full().gap(px(10.)).children(blocks(&r.children, cx)).into_any_element(),
        Node::Definition(_) | Node::FootnoteDefinition(_) | Node::Yaml(_) | Node::Toml(_) => return None,
        other => {
            let children = other.children()?;
            div().w_full().child(inline(children, cx)).into_any_element()
        }
    })
}

/// Render Markdown text. Unparsable input falls back to plain text.
#[allow(clippy::too_many_arguments)]
pub fn render(
    id: SharedString,
    text: &str,
    breaks: Breaks,
    colors: Palette,
    mono: SharedString,
    mono_size: Pixels,
    images: Option<Avatars>,
    base: Option<Base>,
) -> AnyElement {
    let cx = Ctx {
        id,
        next: Cell::new(0),
        colors,
        mono,
        mono_size,
        breaks,
        images,
        base,
    };
    match markdown::to_mdast(text, &markdown::ParseOptions::gfm()) {
        Ok(Node::Root(root)) => col()
            .w_full()
            .gap(px(10.))
            .text_size(px(13.))
            .children(blocks(&root.children, &cx))
            .into_any_element(),
        _ => div().text_size(px(13.)).child(text.to_string()).into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn recognises_suggestion_fences() {
        assert!(super::is_suggestion(Some("suggestion")));
        assert!(super::is_suggestion(Some("suggestion:-0+1")));
        assert!(!super::is_suggestion(Some("rust")));
        assert!(!super::is_suggestion(None));
    }
    // Not `super::*`: gpui's own `test` attribute would shadow the built-in one.
    use super::{Base, Breaks, Img, Inline, Node, Style, html_images, resolve, strip_tags};
    use crate::avatars::Source;
    #[test]
    fn reads_img_tags() {
        let imgs = html_images(r#"<p align="center"><img width="320" alt="Shot" src="https://github.com/user-attachments/assets/x"></p><IMG SRC='a.png'>"#);
        assert_eq!(
            imgs,
            vec![
                Img { url: "https://github.com/user-attachments/assets/x".into(), alt: "Shot".into(), link: None, width: Some(320.) },
                Img { url: "a.png".into(), alt: String::new(), link: None, width: None },
            ]
        );
    }
    #[test]
    fn resolves_relative_images_in_the_repo() {
        let base = Base { slug: "o/r".into(), reference: "main".into(), dir: "docs/guide".into() };
        let repo = |p: &str| Some(Source::Repo { slug: "o/r".into(), reference: "main".into(), path: p.into() });
        assert_eq!(resolve("img/a.png", Some(&base)), repo("docs/guide/img/a.png"));
        assert_eq!(resolve("./../b.png?raw=1", Some(&base)), repo("docs/b.png"));
        assert_eq!(resolve("/assets/c.svg", Some(&base)), repo("assets/c.svg"));
        assert_eq!(resolve("https://x.dev/a.png", None), Some(Source::Url("https://x.dev/a.png".into())));
        assert_eq!(resolve("a.png", None), None);
        assert_eq!(resolve("data:image/png;base64,xx", Some(&base)), None);
    }
    fn flatten(src: &str, breaks: Breaks) -> Inline {
        let Ok(Node::Root(root)) = markdown::to_mdast(src, &markdown::ParseOptions::gfm()) else {
            panic!()
        };
        let Node::Paragraph(p) = &root.children[0] else { panic!() };
        let mut i = Inline::default();
        i.walk(&p.children, &Style::default(), breaks);
        i
    }
    #[test]
    fn styles_inline_code_and_links() {
        let i = flatten("Run `cargo test` or see [the docs](https://x.dev) now", Breaks::Soft);
        assert_eq!(i.text, "Run cargo test or see the docs now");
        assert!(i.runs.iter().any(|(len, s)| s.code && *len == "cargo test".len()));
        assert_eq!(i.links, vec![(22..30, "https://x.dev".to_string())]);
    }
    #[test]
    fn soft_and_hard_breaks() {
        assert_eq!(flatten("one\ntwo", Breaks::Soft).text, "one two");
        assert_eq!(flatten("one\ntwo", Breaks::Hard).text, "one\ntwo");
    }
    #[test]
    fn strips_html_tags() {
        assert_eq!(strip_tags("<details><summary>More</summary> info</details>"), "More info");
    }
}
