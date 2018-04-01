use cursive::Cursive;
use cursive::views::{Button, Dialog, DummyView, LinearLayout, ListView, ProgressBar, TextView};
use cursive::With;
use std::thread;
use std::time::Duration;
use cursive::views::Counter;
use cursive_table_view::{TableView, TableViewItem};
use rand::{thread_rng, Rng};
use client::table;

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

fn push_dialog(s: &mut Cursive) {
    // A little break before things get serious.
    s.add_layer(
        Dialog::new()
            .title("Preparation complete")
            .content(TextView::new("Now, the real deal!").center())
            .button("Close", |s| s.pop_layer()),
    );
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
            cb.send(Box::new(push_dialog)).unwrap();
    });
}

pub fn run() {
    let mut rng = thread_rng();
    let mut curs = Cursive::new(); /*
    let mut manager = PeerManager::new();
    let port = "3000";
    let recv = manager.handle(port);*/

    curs.add_global_callback('q', |s| s.quit());

    let table = table::new();
    /*
    let list = ListView::new().with(|list| {
        for i in 0..20 {
            list.add_child(&format!("Torrent {}", i), progress(&curs));
        }
    });

    //let table_container = LinearLayout::horizontal().child(list).child(table);
    */
    let actions = LinearLayout::horizontal()
        .child(Button::new("New", add))
        .child(Button::new("Remove", remove))
        .child(DummyView)
        .child(Button::new("Quit", Cursive::quit));

    curs.add_layer(
        Dialog::around(LinearLayout::vertical().child(table).child(actions))
            .title("Active Torrents"),
    );

    // Starts the event loop.
    curs.set_fps(1);
    curs.run();
}

fn add(curs: &mut Cursive) {}

fn pause(curs: &mut Cursive) {}

fn remove(curs: &mut Cursive) {}
