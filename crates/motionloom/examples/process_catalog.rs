use motionloom::api::{process_effect_for_id, process_effects};

fn main() {
    for effect in process_effects() {
        println!(
            "{}\t{}\t{:?}\t{}",
            effect.id, effect.display_name, effect.category, effect.summary
        );
    }

    if let Some(effect) = process_effect_for_id("glow_bloom") {
        println!("\nlookup glow_bloom -> {}", effect.display_name);
    }
}
