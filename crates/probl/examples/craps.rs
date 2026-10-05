//! Embedding Probl: run the craps example, enumerated and sampled, and read
//! its reports as numbers.
//!
//! ```sh
//! cargo run -p probl --example craps
//! ```

use probl::{Options, compile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_str!("../../../examples/02_craps.probl");
    let craps = compile("02_craps.probl", source)?;

    let enumerated = craps.run(&Options::new())?;
    let win = enumerated
        .report("win")
        .and_then(|r| r.groups().first())
        .and_then(|g| g.probability())
        .ok_or("no `win` report")?;
    // Close to 244 / 495: the loop stops with a tiny unresolved tail, so the
    // estimate isn't complete, and its bounds hold the exact answer.
    println!("win: {:?} (exact 244/495 = {})", win.point(), 244.0 / 495.0);
    println!("complete: {}", win.is_complete());
    if let Some(bounds) = win.bounds() {
        println!("bounds: {}..{}", bounds.lower(), bounds.upper());
    }
    if let Some(rolls) = enumerated.report("rolls per bet").and_then(|r| r.groups().first()) {
        let numeric = rolls.numeric().ok_or("rolls aren't numbers")?;
        println!(
            "rolls per bet: mean {:?}, median {:?}",
            numeric.mean().and_then(|m| m.point()),
            numeric.quantile(0.5)?.point()
        );
    }

    // The same report, sampled: with its Monte Carlo error.
    let sampled = craps.run(&Options::new().runs(100_000).seed(1))?;
    print!("\n{}", sampled.text());
    Ok(())
}
