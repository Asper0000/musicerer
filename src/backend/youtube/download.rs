use std::process::Command;

pub fn add_args(cmd: &mut Command, fallback_attempt: u8, fragments: usize) {
    let player_client = if fallback_attempt > 0 {
        "android"
    } else {
        "default,web_embedded,tv"
    };

    // Strip common YouTube upload decorations before tagging. The
    // `- Topic` and `VEVO` suffixes are how YouTube Music and label
    // accounts mark their uploads; removing them leaves the bare
    // artist name in `uploader`, which native extraction falls back to
    // when `artist` is empty.
    cmd.arg("--replace-in-metadata")
        .arg("uploader")
        .arg(r" - Topic$")
        .arg("")
        .arg("--replace-in-metadata")
        .arg("uploader")
        .arg(r"VEVO$")
        .arg("");
    for suffix in super::UNWANTED_TITLE_SUFFIXES {
        cmd.arg("--replace-in-metadata")
            .arg("title")
            .arg(*suffix)
            .arg("");
    }

    // Split "Artist - Title" into separate fields when the pattern is
    // present. The regex form is safe when the separator is absent: the
    // title field is left untouched instead of getting a "NA - Title"
    // prefix as the format form (title:%(artist)s - %(title)s) would.
    cmd.arg("--parse-metadata")
        .arg(r"title:(?P<artist>.+?) - (?P<title>.+)$");

    cmd.arg("--format").arg("bestaudio[ext=m4a]/bestaudio/best")
        .arg("--audio-quality").arg("256K")
        .arg("--js-runtimes").arg("deno")
        .arg("--remote-components").arg("ejs:github")
        .arg("--extractor-args").arg(format!("youtube:player_client={player_client}"))
        .arg("--concurrent-fragments").arg(fragments.to_string());
}