use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    style::Print,
    terminal,
};
use std::io::{stdout};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out = stdout();

    terminal::enable_raw_mode()?;

    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(5, 5),
        Print("TUI Working. Press q to quit")
    )?;

    loop {
        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }

    terminal::disable_raw_mode()?;
    Ok(())
}