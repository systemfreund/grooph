//! Writes the guide pages into the web build. Run as a Trunk `post_build`
//! hook (output directory from `TRUNK_STAGING_DIR`) or by hand with the
//! output directory as the only argument. Also writes `embed.json`, the
//! app's file names for the MCP App view. When `GROOPH_BUILD` is set (CI),
//! also writes it to `version.txt`, which the app polls to notice new deploys.

use std::path::{Path, PathBuf};

fn main() {
    let out = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("TRUNK_STAGING_DIR").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("usage: grooph-site <output dir> (or set TRUNK_STAGING_DIR)");
            std::process::exit(2);
        });
    let llms_base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/assets/seo/llms.txt");
    let result = std::fs::read_to_string(&llms_base)
        .map_err(|e| format!("{}: {e}", llms_base.display()))
        .and_then(|base| {
            let pages = grooph_site::load_pages(&grooph_site::content_dir())?;
            let pages = grooph_site::build(&out, &pages, &base)?;
            grooph_site::write_embed_manifest(&out)?;
            if let Ok(build) = std::env::var("GROOPH_BUILD") {
                let path = out.join("version.txt");
                std::fs::write(&path, build).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            Ok(pages)
        });
    match result {
        Ok(pages) => eprintln!("grooph-site: wrote {} pages to {}", pages.len(), out.display()),
        Err(err) => {
            eprintln!("grooph-site: {err}");
            std::process::exit(1);
        }
    }
}
