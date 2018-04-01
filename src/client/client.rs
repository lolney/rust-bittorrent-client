use cursive::Cursive;
use cursive::views::{Button, Dialog, DummyView, LinearLayout, ListView, ProgressBar, TextView};
use cursive::With;
use std::thread;
use std::time::Duration;
use cursive::views::Counter;
use cursive_table_view::{TableView, TableViewItem};

pub struct Client {
    curs: Cursive,
}

fn fake_load(n_max: usize, counter: &Counter) {
    for _ in 0..n_max {
        thread::sleep(Duration::from_secs(1));
        // The `counter.tick()` method increases the progress value
        counter.tick(1);
    }
}

fn coffee_break(s: &mut Cursive) {
    // A little break before things get serious.
    s.add_layer(
        Dialog::new()
            .title("Preparation complete")
            .content(TextView::new("Now, the real deal!").center())
            .button("Quit", |s| s.pop_layer()),
    );
}

pub fn new() -> Client {
    Client {
        curs: Cursive::new(),
    }
}

fn progress(curs: &Cursive) -> ProgressBar {
    let n_max = 10;
    let cb = curs.cb_sink().clone();

    return ProgressBar::new()
        // We need to know how many ticks represent a full bar.
        .range(0, n_max)
        .with_task(move |counter| {
            // This closure will be called in a separate thread.
            fake_load(n_max, &counter);

            // When we're done, send a callback through the channel
            cb.send(Box::new(coffee_break)).unwrap();
    });
}

pub fn run() {
    let mut curs = Cursive::new();
    curs.add_global_callback('q', |s| s.quit());

    let list = ListView::new().with(|list| {
        // We can also add children procedurally
        list.add_child("Torrent", progress(&curs));
        for i in 0..50 {
            list.add_child(&format!("Item {}", i), TextView::new("Hello"));
        }
    });

    let actions = LinearLayout::horizontal()
        .child(Button::new("New", add))
        .child(Button::new("Remove", remove))
        .child(DummyView)
        .child(Button::new("Quit", Cursive::quit));

    curs.add_layer(
        Dialog::around(LinearLayout::vertical().child(list).child(actions))
            .title("Active Torrents"),
    );

    // Starts the event loop.
    curs.set_fps(1);
    curs.run();
}

fn add(curs: &mut Cursive) {}

fn pause(curs: &mut Cursive) {}

fn remove(curs: &mut Cursive) {}
