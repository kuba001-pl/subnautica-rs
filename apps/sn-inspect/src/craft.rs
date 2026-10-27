//! M9e1: resolve every fabricator craft node against the installed data.
use std::process::ExitCode;

use sn_assets::Assets;
use sn_install::GameData;
use sn_sim::craft::{ENERGY, MIN_TIME};

pub fn run(game: &GameData) -> crate::Result<ExitCode> {
    let assets = Assets::index(game)?;
    let code = sn_assets::game_code(&sn_assets::read_assembly(game)?)?;
    let data = sn_assets::tech_data(&assets)?;
    let fabricator = sn_assets::pod_fabricator(game, &assets, [0.0; 3])?;
    println!(
        "fabricator at {:?} itemSpawnPoint {:?} closeDistance={} handText={}",
        fabricator.at.position,
        fabricator.item_spawn.position,
        fabricator.close_distance,
        fabricator.hand_text
    );
    println!(
        "initial pod power={} (regeneration not simulated)",
        sn_assets::pod_crafting_power(game, &assets)?
    );
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
    // M9e3: each output's ghost (CrafterGhostModel.SetupGhostModelAsync).
    let (rules, _) = sn_assets::item_rules(game, &assets, &sn_assets::read_assembly(game)?)?;
    let mut classes: Vec<String> = rules
        .recipes
        .values()
        .flat_map(|r| std::iter::once(r.tech).chain(r.linked.iter().copied()))
        .filter_map(|t| rules.tech_class.get(&t).cloned())
        .collect();
    classes.sort();
    classes.dedup();
    let ghosts = sn_assets::fabricator_ghosts(game, &assets, &classes)?;
    let (mut vfx, mut model, mut none, mut inactive) = (0, 0, 0, 0);
    for (class, ghost) in &ghosts {
        match ghost {
            Some(g) => {
                if g.constructable {
                    model += 1;
                } else {
                    vfx += 1;
                }
                if !g.active {
                    inactive += 1;
                }
                println!(
                    "ghost {class}: node {:?} constructable={} active={} root in itemSpawnPoint {:?} {:?} {:?}",
                    g.node,
                    g.constructable,
                    g.active,
                    g.root.position,
                    g.root.rotation,
                    g.root.scale
                );
            }
            None => {
                none += 1;
                println!("ghost {class}: none (no Constructable.model, no VFXFabricating)");
            }
        }
    }
    println!(
        "ghosts: output classes={} read={} VFXFabricating={vfx} Constructable.model={model} none={none} inactive node={inactive}",
        classes.len(),
        ghosts.len()
    );
    println!("RUN OK");
    Ok(ExitCode::SUCCESS)
}
