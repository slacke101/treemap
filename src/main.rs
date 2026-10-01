mod app;
mod consent;
mod ui;

fn main() -> std::io::Result<()> {
    let mut terminal = ui::setup_terminal()?;
    let result = app::App::new().run(&mut terminal);
    // Always restore the terminal, even if `run` returned an error.
    ui::restore_terminal()?;
    result
}
