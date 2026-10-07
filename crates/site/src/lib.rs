//! Static guide pages for grooph.app: crawlable text around the canvas app.
//!
//! Every Markdown file under `content/` becomes one page; `content/a/b.md` is
//! served at `/a/b/`, `content/a/index.md` at `/a/`. A file starts with front
//! matter between `---` lines:
//!
//! ```text
//! ---
//! title: Online Metronome        (<title> and default <h1>)
//! heading: ...                   (optional <h1>)
//! description: ...               (meta description, listings)
//! section: Metronome             (footer group: Metronome, Sight-reading, Learn, Features)
//! order: 1                       (position within the section)
//! ---
//! ```
//!
//! Fenced code blocks with these info strings are rendered specially:
//!
//! - ` ```rhythm bpm=90 swing=66 | Caption ` with a score in the text notation
//!   of [`grooph_measure::notation`]: a proportional grid with counting
//!   syllables and a link that opens the rhythm in the app.
//! - ` ```open | Label ` with a link query (see [`grooph_link`]): a button
//!   that opens the app with these parameters.
//! - ` ```pages ` with a section name: a list of that section's pages.
//!
//! A `## FAQ` section with `### Question` headings becomes FAQPage
//! structured data. Invalid rhythms or links fail the build, naming the file.

use grooph_link::{LinkContent, SharedLink, parse_query};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::notation::parse_score;
use grooph_measure::swing::{Swing, SwingUnit};
use grooph_measure::{Beat, BeatKind, Measure, TimeSignature};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub const SITE_URL: &str = "https://grooph.app";

/// Footer groups, in display order.
const SECTIONS: [&str; 4] = ["Metronome", "Sight-reading", "Learn", "Features"];

const STYLESHEET: &str = include_str!("site.css");

/// A source page before rendering.
#[derive(Debug, Clone)]
pub struct Page {
    /// URL path with leading and trailing slash, e.g. `/learn/counting-triplets/`.
    pub path: String,
    pub title: String,
    pub heading: String,
    pub description: String,
    pub section: String,
    pub order: i32,
    pub markdown: String,
    /// Source file, for error messages.
    pub source: PathBuf,
}

/// Directory with the Markdown sources.
pub fn content_dir() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("content") }

/// Load every page under `dir`, sorted by section and order.
pub fn load_pages(dir: &Path) -> Result<Vec<Page>, String> {
    let mut files = Vec::new();
    collect_markdown(dir, &mut files)?;
    let mut pages = files
        .iter()
        .map(|file| {
            let text =
                std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let rel = file.strip_prefix(dir).expect("file below content dir");
            parse_page(&text, rel).map_err(|e| format!("{}: {e}", file.display())).map(|mut p| {
                p.source = file.clone();
                p
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    pages.sort_by_key(|p| {
        (SECTIONS.iter().position(|s| *s == p.section).unwrap_or(SECTIONS.len()), p.order)
    });
    Ok(pages)
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_markdown(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// URL path for a content file relative to the content directory.
fn path_for(rel: &Path) -> String {
    let mut parts: Vec<String> =
        rel.with_extension("").iter().map(|c| c.to_string_lossy().into_owned()).collect();
    if parts.last().is_some_and(|p| p == "index") {
        parts.pop();
    }
    if parts.is_empty() { "/".into() } else { format!("/{}/", parts.join("/")) }
}

fn parse_page(text: &str, rel: &Path) -> Result<Page, String> {
    let rest = text.strip_prefix("---\n").ok_or("missing front matter")?;
    let (front, markdown) = rest.split_once("\n---\n").ok_or("unterminated front matter")?;
    let mut page = Page {
        path: path_for(rel),
        title: String::new(),
        heading: String::new(),
        description: String::new(),
        section: String::new(),
        order: 0,
        markdown: markdown.to_string(),
        source: PathBuf::new(),
    };
    for line in front.lines().filter(|l| !l.trim().is_empty()) {
        let (key, value) =
            line.split_once(':').ok_or_else(|| format!("bad front matter '{line}'"))?;
        let value = value.trim().to_string();
        match key.trim() {
            "title" => page.title = value,
            "heading" => page.heading = value,
            "description" => page.description = value,
            "section" => page.section = value,
            "order" => page.order = value.parse().map_err(|_| format!("bad order '{value}'"))?,
            other => return Err(format!("unknown front matter key '{other}'")),
        }
    }
    if page.title.is_empty() || page.description.is_empty() {
        return Err("title and description are required".into());
    }
    if !SECTIONS.contains(&page.section.as_str()) {
        return Err(format!("section must be one of {SECTIONS:?}, got '{}'", page.section));
    }
    if page.heading.is_empty() {
        page.heading = page.title.clone();
    }
    Ok(page)
}

/// Rendered page: final HTML plus what the build needs to know about it.
pub struct Rendered {
    pub path: String,
    pub html: String,
}

/// Render all pages and write them, the stylesheet, `sitemap.xml` and
/// `llms.txt` into `out` (the web build's output directory).
pub fn build(out: &Path, pages: &[Page], llms_base: &str) -> Result<Vec<Rendered>, String> {
    let rendered = pages.iter().map(|p| render_page(p, pages)).collect::<Result<Vec<_>, _>>()?;
    for page in &rendered {
        let dir = out.join(page.path.trim_matches('/'));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        write(&dir.join("index.html"), &page.html)?;
    }
    write(&out.join("site.css"), STYLESHEET)?;
    write(&out.join("sitemap.xml"), &sitemap(pages))?;
    write(&out.join("llms.txt"), &llms_txt(llms_base, pages))?;
    Ok(rendered)
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn sitemap(pages: &[Page]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    let paths = std::iter::once("/").chain(pages.iter().map(|p| p.path.as_str()));
    for path in paths {
        let _ = writeln!(out, "  <url><loc>{SITE_URL}{path}</loc></url>");
    }
    out.push_str("</urlset>\n");
    out
}

/// `llms.txt`: the hand-written base plus a list of the guide pages.
fn llms_txt(base: &str, pages: &[Page]) -> String {
    let mut out = base.trim_end().to_string();
    out.push_str("\n\n## Guides\n\n");
    for page in pages {
        let _ = writeln!(out, "- [{}]({SITE_URL}{}): {}", page.title, page.path, page.description);
    }
    out
}

// ---------------------------------------------------------------- rendering

fn render_page(page: &Page, all: &[Page]) -> Result<Rendered, String> {
    let fail = |e: String| format!("{}: {e}", page.source.display());
    let (body, faq) = render_markdown(&page.markdown, all, &page.path).map_err(fail)?;

    let crumbs = breadcrumbs(page, all);
    let mut crumb_html = String::from("<nav class=\"crumbs\" aria-label=\"Breadcrumb\">");
    for (i, (name, path)) in crumbs.iter().enumerate() {
        if i + 1 < crumbs.len() {
            let _ = write!(crumb_html, "<a href=\"{}\">{}</a> › ", path, esc(name));
        } else {
            let _ = write!(crumb_html, "<span aria-current=\"page\">{}</span>", esc(name));
        }
    }
    crumb_html.push_str("</nav>");

    let url = format!("{SITE_URL}{}", page.path);
    let json_ld = structured_data(page, &url, &crumbs, &faq);
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} – grooph</title>
<meta name="description" content="{description}">
<link rel="canonical" href="{url}">
<meta property="og:type" content="article">
<meta property="og:url" content="{url}">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{description}">
<meta property="og:site_name" content="grooph">
<meta property="og:image" content="{SITE_URL}/assets/preview.png">
<meta name="twitter:card" content="summary_large_image">
<link rel="icon" href="/favicon.ico">
<link rel="stylesheet" href="/site.css">
<script type="application/ld+json">{json_ld}</script>
</head>
<body>
<header class="top">
<a class="brand" href="/">grooph</a>
<nav class="main-nav" aria-label="Guides">
<a href="/metronome/">Metronome</a>
<a href="/rhythm-generator/">Rhythm generator</a>
<a href="/sight-reading/">Sight-reading</a>
<a href="/learn/">Learn</a>
</nav>
<a class="cta" href="/">Open the app</a>
</header>
<main>
{crumb_html}
<h1>{heading}</h1>
{body}
</main>
{footer}
</body>
</html>
"#,
        title = esc(&page.title),
        description = esc(&page.description),
        heading = esc(&page.heading),
        footer = footer(all),
    );
    Ok(Rendered { path: page.path.clone(), html })
}

/// Home, then every existing ancestor page, then the page itself.
fn breadcrumbs(page: &Page, all: &[Page]) -> Vec<(String, String)> {
    let mut crumbs = vec![("grooph".to_string(), "/".to_string())];
    let segments: Vec<&str> = page.path.trim_matches('/').split('/').collect();
    for depth in 1..segments.len() {
        let ancestor = format!("/{}/", segments[..depth].join("/"));
        if let Some(p) = all.iter().find(|p| p.path == ancestor) {
            crumbs.push((p.title.clone(), p.path.clone()));
        }
    }
    crumbs.push((page.title.clone(), page.path.clone()));
    crumbs
}

fn footer(all: &[Page]) -> String {
    let mut out = String::from("<footer>\n<div class=\"groups\">\n");
    for section in SECTIONS {
        let pages: Vec<&Page> = all.iter().filter(|p| p.section == section).collect();
        if pages.is_empty() {
            continue;
        }
        let _ = write!(out, "<div><h2>{}</h2><ul>", esc(section));
        for p in pages {
            let _ = write!(out, "<li><a href=\"{}\">{}</a></li>", p.path, esc(&p.title));
        }
        out.push_str("</ul></div>\n");
    }
    out.push_str(
        "</div>\n<p>grooph is a free metronome and rhythm trainer in your browser. \
         Feedback: <a href=\"mailto:hello@grooph.app\">hello@grooph.app</a></p>\n</footer>",
    );
    out
}

fn structured_data(
    page: &Page,
    url: &str,
    crumbs: &[(String, String)],
    faq: &[(String, String)],
) -> String {
    let items: Vec<String> = crumbs
        .iter()
        .enumerate()
        .map(|(i, (name, path))| {
            format!(
                r#"{{"@type":"ListItem","position":{},"name":{},"item":{}}}"#,
                i + 1,
                json(name),
                json(&format!("{SITE_URL}{path}"))
            )
        })
        .collect();
    let mut graph = vec![
        format!(
            r#"{{"@type":"WebPage","@id":{url},"url":{url},"name":{name},"description":{desc},"inLanguage":"en","isPartOf":{{"@type":"WebSite","name":"grooph","url":"{SITE_URL}/"}}}}"#,
            url = json(url),
            name = json(&page.title),
            desc = json(&page.description),
        ),
        format!(r#"{{"@type":"BreadcrumbList","itemListElement":[{}]}}"#, items.join(",")),
    ];
    if !faq.is_empty() {
        let questions: Vec<String> = faq
            .iter()
            .map(|(q, a)| {
                format!(
                    r#"{{"@type":"Question","name":{},"acceptedAnswer":{{"@type":"Answer","text":{}}}}}"#,
                    json(q),
                    json(a)
                )
            })
            .collect();
        graph.push(format!(r#"{{"@type":"FAQPage","mainEntity":[{}]}}"#, questions.join(",")));
    }
    format!(r#"{{"@context":"https://schema.org","@graph":[{}]}}"#, graph.join(","))
}

/// Markdown to HTML with the custom blocks; also returns the FAQ entries.
/// `current` is the page's own path, left out of `pages` lists.
fn render_markdown(
    md: &str,
    all: &[Page],
    current: &str,
) -> Result<(String, Vec<(String, String)>), String> {
    let events: Vec<Event> = Parser::new_ext(md, Options::ENABLE_TABLES).collect();
    let faq = extract_faq(&events);

    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) = &events[i] {
            let mut body = String::new();
            let mut j = i + 1;
            while !matches!(events[j], Event::End(TagEnd::CodeBlock)) {
                if let Event::Text(t) = &events[j] {
                    body.push_str(t);
                }
                j += 1;
            }
            let (kind, attrs, caption) = split_info(info);
            let html = match kind {
                "rhythm" => rhythm_block(body.trim(), attrs, caption)?,
                "open" => open_block(body.trim(), caption)?,
                "pages" => pages_block(body.trim(), all, current)?,
                _ => String::new(),
            };
            if html.is_empty() {
                out.extend_from_slice(&events[i..=j]);
            } else {
                out.push(Event::Html(html.into()));
            }
            i = j + 1;
            continue;
        }
        out.push(events[i].clone());
        i += 1;
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, out.into_iter());
    Ok((html, faq))
}

/// `rhythm bpm=90 | Caption` → ("rhythm", "bpm=90", "Caption").
fn split_info(info: &str) -> (&str, &str, &str) {
    let (head, caption) = info.split_once('|').unwrap_or((info, ""));
    let head = head.trim();
    let (kind, attrs) = head.split_once(' ').unwrap_or((head, ""));
    (kind, attrs.trim(), caption.trim())
}

/// Questions (`###` under `## FAQ`) with their answer text.
fn extract_faq(events: &[Event]) -> Vec<(String, String)> {
    let mut faq = Vec::new();
    let mut in_faq = false;
    let mut heading: Option<(HeadingLevel, String)> = None;
    let mut answer: Option<String> = None;
    let mut question = String::new();
    for event in events {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if let Some(a) = answer.take() {
                    faq.push((std::mem::take(&mut question), a.trim().to_string()));
                }
                heading = Some((*level, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, text)) = heading.take() {
                    match level {
                        HeadingLevel::H2 => in_faq = text.trim() == "FAQ",
                        HeadingLevel::H3 if in_faq => {
                            question = text.trim().to_string();
                            answer = Some(String::new());
                        }
                        _ => {}
                    }
                }
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, text)) = heading.as_mut() {
                    text.push_str(t);
                } else if let Some(a) = answer.as_mut() {
                    a.push_str(t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(a) = answer.as_mut() {
                    a.push(' ');
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(a) = answer.as_mut() {
                    a.push(' ');
                }
            }
            _ => {}
        }
    }
    if let Some(a) = answer.take() {
        faq.push((question, a.trim().to_string()));
    }
    faq
}

fn parse_attrs(attrs: &str) -> Result<(Option<u32>, Option<Swing>), String> {
    let mut query = Vec::new();
    for attr in attrs.split_whitespace() {
        let (key, _) = attr.split_once('=').ok_or_else(|| format!("bad attribute '{attr}'"))?;
        if !matches!(key, "bpm" | "swing" | "su") {
            return Err(format!("unknown rhythm attribute '{key}'"));
        }
        query.push(attr);
    }
    let link = parse_query(&query.join("&"))?.unwrap_or_default();
    Ok((link.bpm, link.swing))
}

fn rhythm_block(notation: &str, attrs: &str, caption: &str) -> Result<String, String> {
    let score = parse_score(notation).map_err(|e| format!("rhythm '{notation}': {e}"))?;
    let (bpm, swing) = parse_attrs(attrs)?;
    let link = SharedLink { bpm, swing, content: Some(LinkContent::Score(score.clone())) };
    let href = link.to_url("/").ok_or_else(|| format!("rhythm '{notation}' has no link form"))?;

    let mut out = String::from("<figure class=\"rhythm\">\n<div class=\"measures\">");
    for measure in &score.measures {
        out.push_str(&measure_grid(measure));
    }
    out.push_str("</div>\n<figcaption>");
    if !caption.is_empty() {
        let _ = write!(out, "<span>{}</span> ", esc(caption));
    }
    let tempo = match (bpm, swing) {
        (Some(bpm), Some(s)) => format!(" at {bpm} BPM, {}", swing_label(s)),
        (Some(bpm), None) => format!(" at {bpm} BPM"),
        (None, Some(s)) => format!(", {}", swing_label(s)),
        (None, None) => String::new(),
    };
    let _ = write!(out, "<a class=\"play\" href=\"{}\">▶ Play in grooph{}</a>", esc(&href), tempo);
    out.push_str("</figcaption>\n</figure>\n");
    Ok(out)
}

fn swing_label(swing: Swing) -> String {
    let unit = match swing.unit {
        SwingUnit::Eighths => "eighths",
        SwingUnit::Sixteenths => "sixteenths",
    };
    format!("{}% swing on {unit}", swing.percent)
}

/// One measure as a row of cells whose widths follow the durations; notes
/// are dots, rests are dashes, and every cell shows its counting syllable.
fn measure_grid(measure: &Measure) -> String {
    let ts = measure.time_signature();
    let beats = measure.beats();
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    let spoken: Vec<String> = beats
        .iter()
        .zip(&onsets)
        .map(|(b, &t)| {
            let syllable = count_syllable(t, ts);
            match (b.kind, b.accented) {
                (BeatKind::Rest, _) => format!("({syllable})"),
                (BeatKind::Note, true) => format!("{syllable}!"),
                (BeatKind::Note, false) => syllable,
            }
        })
        .collect();
    let mut out = format!(
        "<div class=\"measure\" role=\"img\" aria-label=\"{ts}: {}\"><span class=\"ts\">{}<br>{}</span><div class=\"cells\">",
        esc(&spoken.join(" ")),
        ts.beats,
        ts.beat_unit
    );
    let mut i = 0;
    while i < beats.len() {
        match beats[i].tuplet_group_id {
            Some(id) => {
                let end = beats[i..].iter().position(|b| b.tuplet_group_id != Some(id));
                let end = end.map_or(beats.len(), |n| i + n);
                let ticks: u32 = beats[i..end].iter().map(ticks_of).sum();
                let n = beats[i].duration.as_tuplet_spec().map_or(0, |s| s.n);
                let _ = write!(
                    out,
                    "<span class=\"tuplet\" style=\"flex:{ticks}\"><span class=\"bracket\">{n}</span><span class=\"inner\">"
                );
                for k in i..end {
                    out.push_str(&cell(&beats[k], onsets[k], ts));
                }
                out.push_str("</span></span>");
                i = end;
            }
            None => {
                out.push_str(&cell(&beats[i], onsets[i], ts));
                i += 1;
            }
        }
    }
    out.push_str("</div></div>");
    out
}

fn ticks_of(beat: &Beat) -> u32 { DEFAULT_GRID.ticks_of(&beat.duration).unwrap_or(0) }

fn cell(beat: &Beat, onset: u32, ts: TimeSignature) -> String {
    let class = match (beat.kind, beat.accented) {
        (BeatKind::Rest, _) => "cell rest",
        (BeatKind::Note, true) => "cell note accent",
        (BeatKind::Note, false) => "cell note",
    };
    let on_beat = onset % DEFAULT_GRID.ticks_per_beat(&ts) == 0;
    format!(
        "<span class=\"{class}{}\" style=\"flex:{}\"><i></i><b>{}</b></span>",
        if on_beat { " beat" } else { "" },
        ticks_of(beat),
        esc(&count_syllable(onset, ts))
    )
}

/// Counting syllable of an onset: the beat number on the beat, then
/// "e & a" for sixteenths and "trip let" for triplets.
pub fn count_syllable(onset: u32, ts: TimeSignature) -> String {
    let beat = DEFAULT_GRID.ticks_per_beat(&ts);
    let (n, pos) = (onset / beat + 1, onset % beat);
    match pos {
        0 => n.to_string(),
        p if p * 2 == beat => "&".into(),
        p if p * 4 == beat => "e".into(),
        p if p * 4 == beat * 3 => "a".into(),
        p if p * 3 == beat => "trip".into(),
        p if p * 3 == beat * 2 => "let".into(),
        _ => "·".into(),
    }
}

fn open_block(query: &str, label: &str) -> Result<String, String> {
    let link = parse_query(query)
        .map_err(|e| format!("link '{query}': {e}"))?
        .ok_or_else(|| format!("link '{query}' has no grooph parameters"))?;
    let href = link.to_url("/").ok_or_else(|| format!("link '{query}' has no URL form"))?;
    let label = if label.is_empty() { "Open in grooph" } else { label };
    Ok(format!(
        "<p class=\"open\"><a class=\"button\" href=\"{}\">▶ {}</a></p>\n",
        esc(&href),
        esc(label)
    ))
}

fn pages_block(section: &str, all: &[Page], current: &str) -> Result<String, String> {
    let pages: Vec<&Page> =
        all.iter().filter(|p| p.section == section && p.path != current).collect();
    if pages.is_empty() {
        return Err(format!("no pages in section '{section}'"));
    }
    let mut out = String::from("<ul class=\"pages\">\n");
    for p in pages {
        let _ = writeln!(
            out,
            "<li><a href=\"{}\">{}</a><br><span>{}</span></li>",
            p.path,
            esc(&p.title),
            esc(&p.description)
        );
    }
    out.push_str("</ul>\n");
    Ok(out)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// JSON string literal.
fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            // Keep `</script>` from ending the JSON-LD block.
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests;
