use super::*;

fn page(path: &str, section: &str, markdown: &str) -> Page {
    Page {
        path: path.into(),
        title: format!("Title of {path}"),
        heading: format!("Heading of {path}"),
        description: "A page.".into(),
        section: section.into(),
        order: 0,
        markdown: markdown.into(),
        source: PathBuf::from(format!("content{path}index.md")),
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grooph-site-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every `href="..."` in `html`.
fn hrefs(html: &str) -> Vec<String> {
    html.split("href=\"").skip(1).map(|s| s[..s.find('"').unwrap()].replace("&amp;", "&")).collect()
}

#[test]
fn counting_syllables() {
    let four = TimeSignature::FOUR_FOUR;
    let beat = DEFAULT_GRID.ticks_per_beat(&four);
    assert_eq!(count_syllable(0, four), "1");
    assert_eq!(count_syllable(beat * 2, four), "3");
    assert_eq!(count_syllable(beat / 4, four), "e");
    assert_eq!(count_syllable(beat / 2, four), "&");
    assert_eq!(count_syllable(beat * 3 / 4, four), "a");
    assert_eq!(count_syllable(beat + beat / 3, four), "trip");
    assert_eq!(count_syllable(beat + beat * 2 / 3, four), "let");
    assert_eq!(count_syllable(beat / 8, four), "·");
    let eight = TimeSignature::SEVEN_EIGHT;
    assert_eq!(count_syllable(DEFAULT_GRID.ticks_per_beat(&eight) * 6, eight), "7");
}

#[test]
fn paths_follow_the_content_tree() {
    assert_eq!(path_for(Path::new("index.md")), "/");
    assert_eq!(path_for(Path::new("metronome/index.md")), "/metronome/");
    assert_eq!(path_for(Path::new("learn/counting-triplets.md")), "/learn/counting-triplets/");
    assert_eq!(path_for(Path::new("rhythm-generator.md")), "/rhythm-generator/");
}

#[test]
fn front_matter_is_validated() {
    let ok = "---\ntitle: T\ndescription: D\nsection: Learn\norder: 2\n---\nBody";
    let p = parse_page(ok, Path::new("learn/x.md")).unwrap();
    assert_eq!((p.title.as_str(), p.heading.as_str(), p.order), ("T", "T", 2));
    assert_eq!(p.markdown, "Body");
    assert!(parse_page("Body", Path::new("x.md")).unwrap_err().contains("front matter"));
    let bad_section = "---\ntitle: T\ndescription: D\nsection: Misc\n---\n";
    assert!(parse_page(bad_section, Path::new("x.md")).unwrap_err().contains("section"));
    let unknown = "---\ntitle: T\ndescription: D\nsection: Learn\ncolour: red\n---\n";
    assert!(parse_page(unknown, Path::new("x.md")).unwrap_err().contains("colour"));
}

#[test]
fn rhythm_block_renders_grid_counting_and_link() {
    let md = "```rhythm bpm=90 swing=66 | Offbeat eighths\n4/4 q e e t8 r:t8 t8 >q\n```\n";
    let (html, _) = render_markdown(md, &[], "/").unwrap();
    assert!(html.contains("class=\"cell note beat\""), "{html}");
    assert!(html.contains("<b>trip</b>"), "{html}");
    assert!(html.contains("class=\"tuplet\""), "{html}");
    assert!(html.contains("class=\"cell note accent beat\""), "{html}");
    assert!(html.contains("aria-label=\"4/4: 1 2 &amp; 3 (trip) let 4!\""), "{html}");
    assert!(html.contains("Offbeat eighths"));
    assert!(html.contains("at 90 BPM, 66% swing on eighths"));
    let href = hrefs(&html).pop().unwrap();
    let link = parse_query(href.trim_start_matches('/')).unwrap().unwrap();
    assert_eq!(link.bpm, Some(90));
    assert!(matches!(link.content, Some(LinkContent::Score(_))));
}

#[test]
fn invalid_blocks_fail() {
    let short = "```rhythm\nq q q\n```\n";
    assert!(render_markdown(short, &[], "/").unwrap_err().contains("too short"));
    let bad_attr = "```rhythm tempo=90\nq q q q\n```\n";
    assert!(render_markdown(bad_attr, &[], "/").unwrap_err().contains("tempo"));
    let bad_link = "```open\nsub=16&lvl=9\n```\n";
    assert!(render_markdown(bad_link, &[], "/").unwrap_err().contains("lvl"));
    let empty_link = "```open\nfoo=1\n```\n";
    assert!(render_markdown(empty_link, &[], "/").unwrap_err().contains("no grooph parameters"));
    let empty_section = "```pages\nLearn\n```\n";
    assert!(render_markdown(empty_section, &[], "/").unwrap_err().contains("Learn"));
}

#[test]
fn open_block_normalizes_the_link() {
    let md = "```open | Level 3\nseed=42&lvl=3&sub=16&bpm=80\n```\n";
    let (html, _) = render_markdown(md, &[], "/").unwrap();
    assert!(
        html.contains("href=\"/?bpm=80&amp;sub=16&amp;lvl=3&amp;bars=1&amp;seed=42\""),
        "{html}"
    );
    assert!(html.contains("▶ Level 3"));
}

#[test]
fn ordinary_code_blocks_stay_code() {
    let (html, _) = render_markdown("```\nq e e\n```\n", &[], "/").unwrap();
    assert!(html.contains("<pre><code>q e e"), "{html}");
}

#[test]
fn faq_becomes_structured_data() {
    let md = "## Intro\n\nText.\n\n## FAQ\n\n### Is it free?\n\nYes, *completely*.\n\n### Offline?\n\nNo.\n\n## More\n\n### Not a question\n\nText.";
    let (_, faq) = render_markdown(md, &[], "/").unwrap();
    assert_eq!(
        faq,
        vec![
            ("Is it free?".to_string(), "Yes, completely.".to_string()),
            ("Offline?".to_string(), "No.".to_string())
        ]
    );
    let p = page("/x/", "Learn", md);
    let json_ld = structured_data(&p, "https://grooph.app/x/", &breadcrumbs(&p, &[]), &faq);
    assert!(json_ld.contains(r#""@type":"FAQPage""#));
    assert!(json_ld.contains(r#""name":"Is it free?""#));
}

#[test]
fn json_strings_are_escaped() {
    let escaped = json("a \"b\" \\ </script>\n");
    assert_eq!(escaped, ["\"a \\\"b\\\" \\\\ ", "\\u003c", "/script>\\n\""].concat());
}

#[test]
fn breadcrumbs_include_existing_ancestors() {
    let hub = page("/learn/", "Learn", "");
    let child = page("/learn/triplets/", "Learn", "");
    let orphan = page("/sight-reading/eighths/", "Sight-reading", "");
    let all = [hub.clone(), child.clone(), orphan.clone()];
    let names: Vec<String> = breadcrumbs(&child, &all).into_iter().map(|(_, p)| p).collect();
    assert_eq!(names, ["/", "/learn/", "/learn/triplets/"]);
    let names: Vec<String> = breadcrumbs(&orphan, &all).into_iter().map(|(_, p)| p).collect();
    assert_eq!(names, ["/", "/sight-reading/eighths/"]);
}

/// The real content: every page builds, and every link points to a page or
/// is an app link the app can open.
#[test]
fn content_builds_and_links_resolve() {
    let pages = load_pages(&content_dir()).unwrap();
    assert!(!pages.is_empty());
    let out = temp_dir("content");
    let rendered = build(&out, &pages, "# grooph\n").unwrap();

    let known: Vec<&str> =
        std::iter::once("/").chain(pages.iter().map(|p| p.path.as_str())).collect();
    for page in &rendered {
        assert!(out.join(page.path.trim_matches('/')).join("index.html").is_file());
        for href in hrefs(&page.html) {
            if let Some(query) = href.strip_prefix("/?") {
                let link = parse_query(query);
                assert!(
                    matches!(link, Ok(Some(_))),
                    "{}: bad app link {href}: {link:?}",
                    page.path
                );
            } else if href.starts_with('/') {
                let ok = known.contains(&href.as_str())
                    || ["/site.css", "/favicon.ico"].contains(&href.as_str());
                assert!(ok, "{}: link to missing page {href}", page.path);
            } else {
                assert!(
                    href.starts_with("https://") || href.starts_with("mailto:"),
                    "{}: {href}",
                    page.path
                );
            }
        }
    }

    let llms = std::fs::read_to_string(out.join("llms.txt")).unwrap();
    let sitemap = std::fs::read_to_string(out.join("sitemap.xml")).unwrap();
    for page in &pages {
        assert!(
            llms.contains(&format!("{SITE_URL}{}", page.path)),
            "llms.txt misses {}",
            page.path
        );
        assert!(sitemap.contains(&format!("<loc>{SITE_URL}{}</loc>", page.path)));
        assert!(
            page.description.len() <= 170,
            "{}: description too long for search results",
            page.path
        );
    }
    let _ = std::fs::remove_dir_all(&out);
}

/// The app's `<noscript>` links point at pages that exist.
#[test]
fn app_page_links_to_existing_guides() {
    let index = include_str!("../../app/index.html");
    let pages = load_pages(&content_dir()).unwrap();
    let guide_links: Vec<String> =
        hrefs(index).into_iter().filter(|h| h.starts_with("/") && h.len() > 1).collect();
    assert!(!guide_links.is_empty());
    for href in guide_links {
        assert!(pages.iter().any(|p| p.path == href), "index.html links to missing page {href}");
    }
}
