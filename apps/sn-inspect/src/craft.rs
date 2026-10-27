//! M9e1: resolve every fabricator craft node against the installed data.
use std::process::ExitCode;

use sn_assets::Assets;
use sn_install::GameData;
use sn_sim::craft::{ENERGY, MIN_TIME};

pub fn run(game: &GameData) -> crate::Result<ExitCode> {
    let assets = Assets::index(game)?;
    let code = sn_assets::game_code(&sn_assets::read_assembly(game)?)?;
    let data = sn_assets::tech_data(&assets)?;
    let player = sn_assets::player_data(&assets)?;
    let tree = code
        .craft_trees
        .iter()
        .find(|t| t.id == "Fabricator")
        .ok_or("no Fabricator tree")?;
    let action = code.tree_action("Craft").ok_or("no Craft action")?;
    let mut nodes = 0;
    let mut unresolved = 0;
    let mut default_time = 0;
    for node in tree.root.walk().into_iter().filter(|n| n.action == action) {
        nodes += 1;
        let Some(entry) = data.get(node.tech_type) else {
            unresolved += 1;
            println!("UNRESOLVED tech={}", node.tech_type);
            continue;
        };
        let recipe = sn_assets::craft_recipe(entry, &code.tech_defaults)?;
        if recipe.time.is_none() {
            default_time += 1;
        }
        println!(
            "{} tech={} ingredients={:?} amount={} linked={:?} time={:?} animation={} duration={} energy={ENERGY}",
            code.tech_name(recipe.tech).unwrap_or("?"),
            recipe.tech,
            recipe.ingredients,
            recipe.amount,
            recipe.linked,
            recipe.time,
            recipe.animation_time(),
            recipe.duration()
        );
        for tech in recipe
            .ingredients
            .iter()
            .map(|i| i.0)
            .chain(recipe.linked.iter().copied())
            .chain([recipe.tech])
        {
            if code.tech_name(tech).is_none() {
                unresolved += 1;
            }
        }
    }
    println!(
        "PDAData starting blueprints={} unique={}",
        player.pda.default_tech.len(),
        player
            .pda
            .default_tech
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
    println!(
        "default time: TechData={} (GetCraftTime returns false when absent); GhostCrafter={MIN_TIME}; CrafterLogic end delay={}",
        code.tech_defaults.craft_time,
        sn_sim::craft::END_DELAY
    );
    let total = tree.root.walk().len();
    println!(
        "fabricator nodes={total} craft nodes={nodes} absent craftTime={default_time} unresolved={unresolved}"
    );
    if total != 100 || nodes != 87 || unresolved != 0 {
        return Err("fabricator recipe checks failed".into());
    }
    println!("RUN OK");
    Ok(ExitCode::SUCCESS)
}
