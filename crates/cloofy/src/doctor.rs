use anyhow::Result;
use cloofy_core::{config::Config, paths::Paths};
use cloofy_spotify::{
    doctor::{self, Check, Expect, Outcome, Section},
    session,
    webapi::WebApi,
};

use crate::style;

const NAME_WIDTH: usize = 30;

pub async fn run(paths: &Paths) -> Result<()> {
    println!("\n  {}\n", style::bold("cloofy doctor"));
    let mut tally = Tally::default();

    let config = Config::load(paths)?;
    let api = WebApi::new(config.client_id(paths)?, paths.webapi_token())?;
    let web = if api.is_logged_in().await { doctor::web_api(&api).await } else { not_logged_in("WEB API · your app") };
    print_section(&web, &mut tally);

    let internal = match session::prepare(paths) {
        Ok(pending) => match pending.connect().await {
            Ok(session) => {
                let section = doctor::spclient(&session).await;
                session.shutdown();
                section
            }
            Err(err) => failed_section("LIBRESPOT · Spotify internal", "Session", format!("{err:#}")),
        },
        Err(_) => not_logged_in("LIBRESPOT · Spotify internal"),
    };
    print_section(&internal, &mut tally);
    print_section(&cloofy_core::doctor::external().await, &mut tally);

    println!(
        "  {} available · {} removed as expected · {}\n",
        style::green(&tally.available.to_string()),
        style::dim(&tally.removed.to_string()),
        if tally.problems == 0 {
            style::green("no problems")
        } else {
            style::red(&format!("{} problems", tally.problems))
        }
    );
    Ok(())
}

#[derive(Default)]
struct Tally {
    available: usize,
    removed: usize,
    problems: usize,
}

fn print_section(section: &Section, tally: &mut Tally) {
    println!("  {}", style::dim(section.title));
    for check in &section.checks {
        print_check(check, tally);
    }
    println!();
}

fn print_check(check: &Check, tally: &mut Tally) {
    let name = format!("{:<NAME_WIDTH$}", check.name);
    let line = match (&check.outcome, check.expect) {
        (Outcome::Ok(detail), Expect::Available) => {
            tally.available += 1;
            format!("{}  {name}{}", style::green("✓"), style::dim(detail))
        }
        (Outcome::Ok(detail), Expect::Removed) => {
            tally.available += 1;
            format!("{}  {name}{}", style::cyan("✓"), style::cyan(&format!("available after all · {detail}")))
        }
        (Outcome::Failed(reason), Expect::Removed) => {
            tally.removed += 1;
            style::dim(&format!("–  {name}removed · {reason}"))
        }
        (Outcome::Failed(reason), Expect::Available) => {
            tally.problems += 1;
            format!("{}  {name}{}", style::red("✗"), style::red(reason))
        }
    };
    println!("  {line}");
}

fn not_logged_in(title: &'static str) -> Section {
    failed_section(title, "Login", "not logged in – run `cloofy login`".into())
}

fn failed_section(title: &'static str, name: &str, reason: String) -> Section {
    Section {
        title,
        checks: vec![Check { name: name.into(), expect: Expect::Available, outcome: Outcome::Failed(reason) }],
    }
}
