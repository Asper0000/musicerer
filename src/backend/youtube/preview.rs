use std::process::Command;

pub fn add_args(cmd: &mut Command) {
    // Strip decorations so the queue shows a cleaner title. The
    // `--parse-metadata` split used by downloads is intentionally
    // omitted here: the queue displays a single "Artist - Title"
    // string, and splitting it into separate fields would drop the
    // artist from the preview.
    for suffix in super::UNWANTED_TITLE_SUFFIXES {
        cmd.arg("--replace-in-metadata")
            .arg("title")
            .arg(*suffix)
            .arg("");
    }

    cmd.arg("--js-runtimes").arg("deno")
        .arg("--remote-components").arg("ejs:github")
        .arg("--extractor-args")
        .arg("youtube:player_client=default,web_embedded,tv");
}