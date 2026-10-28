use anyhow::{Result, bail};

pub const HELP: &str =
    "Agent guidance: install skills globally with `npx skills add refinio/sources --global`.
If skill installation is unavailable or not allowed, run `sources skills` for an overview,
then `sources skills <name>` to read a skill's full instructions without installing it.";

struct Skill {
    name: &'static str,
    summary: &'static str,
    content: &'static str,
}

const SKILLS: &[Skill] = &[
    Skill {
        name: "sources-setup",
        summary: "Set up external checkouts, development links, pnpm dependencies, and Turbo integration.",
        content: include_str!("../skills/sources-setup/SKILL.md"),
    },
    Skill {
        name: "sources-release",
        summary: "Pin Git or package releases, reproduce release inputs, and return to development.",
        content: include_str!("../skills/sources-release/SKILL.md"),
    },
];

pub fn execute(name: Option<&str>) -> Result<()> {
    if let Some(name) = name {
        let Some(skill) = SKILLS.iter().find(|skill| skill.name == name) else {
            bail!("unknown skill {name:?}; run sources skills to list available skills");
        };
        print!("{}", skill.content);
    } else {
        println!("Bundled agent skills:\n");
        for skill in SKILLS {
            println!("  {}\n    {}", skill.name, skill.summary);
        }
        println!("\n{HELP}");
    }
    Ok(())
}
