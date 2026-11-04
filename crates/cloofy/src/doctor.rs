//! `cloofy doctor`: what each music service in use can reach, and through which path.

use anyhow::{Result, bail};
use cloofy_core::{
    doctor::{self, Check, Expect, Outcome, Section},
    paths::Paths,
    service::Service,
};

use crate::style;

const NAME_WIDTH: usize = 30;

pub async fn run(paths: &Paths, services: &[Service]) -> Result<()> {
    println!("\n  {}\n", style::bold("cloofy doctor"));
    let mut tally = Tally::default();

    for &service in services {
        let report = |section: Section| print_section(&section, &mut tally);
        match service {
            #[cfg(feature = "spotify")]
            Service::Spotify => {
                crate::spotify::doctor(paths, &cloofy_core::config::Config::load(paths)?, report).await?
            }
            #[cfg(feature = "ytmusic")]
            Service::YouTubeMusic => crate::ytmusic::doctor(paths, report).await?,
            #[allow(unreachable_patterns)]
            other => bail!("{} isn't built into this cloofy", other.name()),
        }
    }
    print_section(&doctor::external().await, &mut tally);

    // Only Spotify removes what used to be there.
    let removed =
        (tally.removed > 0).then(|| format!(" · {} removed as expected", style::dim(&tally.removed.to_string())));
    println!(
        "  {} available{} · {}\n",
        style::green(&tally.available.to_string()),
        removed.unwrap_or_default(),
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
        (Outcome::Ok(detail), Expect::Optional) => {
            tally.available += 1;
            format!("{}  {name}{}", style::green("✓"), style::dim(detail))
        }
        (Outcome::Failed(reason), Expect::Removed) => {
            tally.removed += 1;
            style::dim(&format!("–  {name}removed · {reason}"))
        }
        (Outcome::Failed(reason), Expect::Optional) => style::dim(&format!("–  {name}{reason}")),
        (Outcome::Failed(reason), Expect::Available) => {
            tally.problems += 1;
            format!("{}  {name}{}", style::red("✗"), style::red(reason))
        }
    };
    println!("  {line}");
}
