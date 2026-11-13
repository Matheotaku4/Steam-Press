//! cs.rin.ru release notes: built-in SuperSteamPacker renders and editable tokens.

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateDepot {
    pub depot_id: String,
    pub depot_name: String,
    pub manifest_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateData {
    pub game_name: String,
    pub os: String,
    pub branch: String,
    pub build_id: String,
    pub build_datetime: String,
    pub upload_datetime: String,
    pub app_id: String,
    pub description: String,
    pub website: Option<String>,
    pub username: String,
    pub filehost: String,
    pub upload_date: String,
    pub depots: Vec<TemplateDepot>,
}

impl TemplateData {
    pub fn depot_lines(&self) -> Vec<String> {
        self.depots
            .iter()
            .map(|depot| {
                let name = if depot.depot_name.trim().is_empty() {
                    "DepotName"
                } else {
                    depot.depot_name.trim()
                };
                format!(
                    "{} - {} [Manifest {}]",
                    depot.depot_id, name, depot.manifest_id
                )
            })
            .collect()
    }

    fn depot_list(&self) -> String {
        self.depot_lines().join("\n")
    }

    pub fn primary_depot(&self) -> Option<&TemplateDepot> {
        self.depots.first()
    }

    fn display_name(&self) -> String {
        self.game_name.replace(['_', '.'], " ")
    }
}

/// `February 24, 2025 - 22:02:36 UTC`, the timestamp style used by templates.
pub fn format_utc_datetime(when: DateTime<Utc>) -> String {
    format!(
        "{} {}, {} - {:02}:{:02}:{:02} UTC",
        month_name(when.month()),
        when.day(),
        when.year(),
        when.hour(),
        when.minute(),
        when.second()
    )
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

fn missing_description() -> &'static str {
    "A description could not be found for this game. Please add this manually."
}

pub fn render_builtin_rin(data: &TemplateData) -> String {
    let game = data.display_name();
    format!(
        "[url=][color=white][b]{game} [{os}] [Branch: {branch}] (Clean Steam Files)[/b][/color][/url]\n\
[size=85][color=white][b]Version:[/b] [i]{date} [Build {build}][/i][/color][/size]\n\n\
[spoiler=\"[color=white]Depots & Manifests[/color]\"][code=text]{depots}[/code][/spoiler]\
[color=white][b]Uploaded version:[/b] [i]{date} [Build {build}][/i][/color]",
        os = data.os,
        branch = data.branch,
        date = data.build_datetime,
        build = data.build_id,
        depots = data.depot_list(),
    )
}

pub fn render_builtin_crew(data: &TemplateData) -> String {
    let game = data.display_name();
    let description = if data.description.trim().is_empty() {
        missing_description()
    } else {
        data.description.trim()
    };
    let website_line = data
        .website
        .as_deref()
        .map(|website| format!("[url]{website}[/url]\n"))
        .unwrap_or_default();
    let host = data.filehost.trim();
    let uploader = data.username.trim();

    format!(
        "[img]https://steamcdn-a.akamaihd.net/steam/apps/{appid}/header.jpg[/img]\n\n\n\
[color=red][b]About This Game:[/b][/color]\n\
[img]https://steamstore-a.akamaihd.net/public/images/v6/maincol_gradient_rule.png[/img]\n\
{description}\n\n\
[color=red][b]Official Site:[/b][/color]\n\
[url]https://store.steampowered.com/app/{appid}/[/url]\n\
{website_line}\n\
[color=red][b]Download Links:[/b][/color]\n\
[list][color=yellow][b]Mirror 1[/b][/color]\n\
[url=][color=cyan]{game}[/color] | [color=#FF8000]#UploadDate#[/color][/url] ({host}) [i]< uploaded by {uploader} / pw: {password} >[/i][/list]\n\n\n\
[color=red][b]{game}[/b][/color]\n\
[spoiler=\"[color=white]Depots & Manifests[/color]\"][code=text]{depots}[/code][/spoiler]\
[color=white][b]Uploaded version:[/b] [i]{date} [Build {build}][/i][/color]",
        appid = data.app_id,
        game = game,
        host = host,
        uploader = uploader,
        password = "cs.rin.ru",
        depots = data.depot_list(),
        date = data.build_datetime,
        build = data.build_id,
    )
}

fn tokens(data: &TemplateData, depot: Option<&TemplateDepot>) -> Vec<(String, String)> {
    let primary = data.primary_depot();
    let mut tokens = vec![
        ("{{game_name}}".to_string(), data.game_name.clone()),
        ("{{os}}".to_string(), data.os.clone()),
        ("{{branch}}".to_string(), data.branch.clone()),
        ("{{build_id}}".to_string(), data.build_id.clone()),
        (
            "{{build_datetime_utc}}".to_string(),
            data.build_datetime.clone(),
        ),
        (
            "{{upload_datetime_utc}}".to_string(),
            data.upload_datetime.clone(),
        ),
        ("{{app_id}}".to_string(), data.app_id.clone()),
        ("{{description}}".to_string(), data.description.clone()),
        (
            "{{website}}".to_string(),
            data.website.clone().unwrap_or_default(),
        ),
        ("{{username}}".to_string(), data.username.clone()),
        ("{{filehost}}".to_string(), data.filehost.clone()),
        ("{{upload_date}}".to_string(), data.upload_date.clone()),
        ("{{depot_list}}".to_string(), data.depot_list()),
        (
            "{{primary_depot_id}}".to_string(),
            primary
                .map(|depot| depot.depot_id.clone())
                .unwrap_or_default(),
        ),
        (
            "{{primary_manifest_id}}".to_string(),
            primary
                .map(|depot| depot.manifest_id.clone())
                .unwrap_or_default(),
        ),
    ];

    if let Some(depot) = depot {
        tokens.push(("{{depot_id}}".to_string(), depot.depot_id.clone()));
        tokens.push(("{{depot_name}}".to_string(), depot.depot_name.clone()));
        tokens.push(("{{manifest_id}}".to_string(), depot.manifest_id.clone()));
    }

    tokens
}

fn replace_tokens(text: &str, data: &TemplateData, depot: Option<&TemplateDepot>) -> String {
    let mut output = text.to_string();
    for (token, value) in tokens(data, depot) {
        output = output.replace(&token, &value);
    }
    output
}

/// Renders an editable template. Unknown tokens are left untouched.
pub fn render_custom(template: &str, data: &TemplateData) -> String {
    const OPEN: &str = "{{#depots}}";
    const CLOSE: &str = "{{/depots}}";

    let Some(start) = template.find(OPEN) else {
        return replace_tokens(template, data, None);
    };
    let before = &template[..start];
    let after = &template[start + OPEN.len()..];
    let Some(end) = after.find(CLOSE) else {
        return replace_tokens(template, data, None);
    };

    let block = &after[..end];
    let tail = &after[end + CLOSE.len()..];
    let mut output = replace_tokens(before, data, None);
    for depot in &data.depots {
        output.push_str(&replace_tokens(block, data, Some(depot)));
    }
    output.push_str(&render_custom(tail, data));
    output
}

pub fn info_file_name(stem: &str, crew: bool) -> String {
    if crew {
        format!("[Upload Crew Info] {stem}.txt")
    } else {
        format!("[CS.RIN.RU Info] {stem}.txt")
    }
}

/// Writes the release notes next to the archive.
pub fn write_info_files(
    directory: &Path,
    stem: &str,
    data: &TemplateData,
    upload_crew: bool,
    custom_template: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;

    let main_content = match custom_template {
        Some(template) => render_custom(template, data),
        None => render_builtin_rin(data),
    };
    let main_path = directory.join(info_file_name(stem, false));
    fs::write(&main_path, main_content).map_err(|error| error.to_string())?;

    let mut paths = vec![main_path];
    if upload_crew {
        let crew_path = directory.join(info_file_name(stem, true));
        fs::write(&crew_path, render_builtin_crew(data)).map_err(|error| error.to_string())?;
        paths.push(crew_path);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_template_data() -> TemplateData {
        TemplateData {
            game_name: "Balatro".into(),
            os: "Linux64".into(),
            branch: "public".into(),
            build_id: "18674832".into(),
            build_datetime: "February 24, 2025 - 18:21:58 UTC".into(),
            upload_datetime: "November 13, 2026 - 12:00:00 UTC".into(),
            app_id: "2379780".into(),
            description: "The poker roguelike.".into(),
            website: Some("https://www.playbalatro.com".into()),
            username: "alice".into(),
            filehost: "example.org".into(),
            upload_date: "2026-11-13".into(),
            depots: vec![TemplateDepot {
                depot_id: "2379781".into(),
                depot_name: "Balatro".into(),
                manifest_id: "4851806656204679952".into(),
            }],
        }
    }

    #[test]
    fn renders_supersteampacker_template() {
        let data = sample_template_data();
        let text = render_builtin_rin(&data);
        assert!(text.contains("[b]Balatro [Linux64] [Branch: public] (Clean Steam Files)[/b]"));
        assert!(text.contains("[Build 18674832]"));
        assert!(text.contains("2379781 - Balatro [Manifest 4851806656204679952]"));
    }

    #[test]
    fn renders_crew_template_with_store_data() {
        let text = render_builtin_crew(&sample_template_data());
        assert!(text.contains("apps/2379780/header.jpg"));
        assert!(text.contains("Official Site"));
        assert!(text.contains("https://www.playbalatro.com"));
        assert!(text.contains("uploaded by alice"));
    }

    #[test]
    fn renders_custom_tokens_and_depot_loop() {
        let data = sample_template_data();
        let template =
            "{{game_name}}/{{build_id}}\n{{#depots}}{{depot_id}}={{manifest_id}}\n{{/depots}}";
        assert_eq!(
            render_custom(template, &data),
            "Balatro/18674832\n2379781=4851806656204679952\n"
        );
    }

    #[test]
    fn unknown_tokens_stay_literal() {
        let data = sample_template_data();
        assert_eq!(render_custom("{{unknown}}", &data), "{{unknown}}");
    }

    #[test]
    fn writes_info_files() {
        let dir = crate::test_support::temp_dir("templates");
        let data = sample_template_data();
        let paths =
            write_info_files(&dir, "Balatro.Build.1.Linux64.public", &data, true, None).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("[CS.RIN.RU Info]"));
        assert!(paths[1]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("[Upload Crew Info]"));
    }

    #[test]
    fn formats_utc_datetimes() {
        use chrono::TimeZone;
        let when = Utc.with_ymd_and_hms(2026, 11, 13, 9, 5, 3).unwrap();
        assert_eq!(
            format_utc_datetime(when),
            "November 13, 2026 - 09:05:03 UTC"
        );
    }
}
