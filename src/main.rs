extern crate cursive;

use cursive::Cursive;
use cursive::views::{Dialog, TextView};

fn main() {
    let mut siv = Cursive::new();

    siv.add_global_callback('q', |s| s.quit());

    siv.add_layer(
        Dialog::around(TextView::new("Hello Dialog!"))
            .title("Cursive")
            .button("New", |s| new(s))
            .button("Pause", |s| pause(s))
            .button("Remove", |s| remove(s))
            .button("Quit", |s| s.quit()),
    );

    // Starts the event loop.
    siv.run();
}

fn new(curs: &Cursive) {}

fn pause(curs: &Cursive) {}

fn remove(curs: &Cursive) {}
