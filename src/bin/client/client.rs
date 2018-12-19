use bittorrent::bittorrent::manager::Manager;
use cursive::traits::*;
use cursive::views::Counter;
use cursive::views::{Button, Dialog, DummyView, LinearLayout, ProgressBar, TextView};
use cursive::Cursive;
use std::cell::RefCell;
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use table;
use table::AsyncTableView;

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
    let mut curs = Cursive::new();

    curs.add_global_callback('q', |s| s.quit());

    let mut manager = Rc::new(RefCell::new(Manager::new()));
    let comm = manager.borrow_mut().handle();

    let async_table = AsyncTableView::new(comm, table::new());
    let actions = LinearLayout::horizontal()
        .child(Button::new("New", move |_| add(manager.clone())))
        .child(Button::new("Remove", remove))
        .child(DummyView)
        .child(Button::new("Quit", Cursive::quit));

    curs.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(async_table.with_id("table").min_size((50, 20)))
                .child(actions),
        )
        .title("Active Torrents"),
    );

    // Starts the event loop.
    curs.set_fps(1);
    curs.run();
}

fn add(mut manager: Rc<RefCell<Manager>>) {
    manager.borrow_mut().add_torrent(
        "/path/to/torrent_file.torrent",
        "/path/to/download/directory",
    );
}

fn pause(curs: &mut Cursive) {}

fn remove(curs: &mut Cursive) {}
/*
#[cfg(test)]
mod tests {

    use client::client::*;

    #[test]
    fn test_client() {
        let mut curs = Cursive::new();
    }
}*/
