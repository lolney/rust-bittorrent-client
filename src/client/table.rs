use std::cmp::Ordering;

use cursive::Cursive;
use cursive::views::{BoxView, IdView};
use cursive::traits::*;
use cursive::align::HAlign;
use cursive::views::{Dialog, TextView};

use cursive_table_view::{TableColumn, TableView, TableViewItem};

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub enum TorrentColumn {
    Name,
    Status,
    Progress,
    Up,
    Down,
    NPeers,
}

impl TorrentColumn {
    fn as_str(&self) -> &str {
        match *self {
            TorrentColumn::Name => "Name",
            TorrentColumn::Status => "Count",
            TorrentColumn::Progress => "Progress",
            TorrentColumn::Up => "Up",
            TorrentColumn::Down => "Down",
            TorrentColumn::NPeers => "#Peers",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Foo {
    name: String,
    status: usize,
    progress: usize,
    up: usize,
    down: usize,
    npeers: usize,
}

// too much boilerplate here, but hard to do this by macro
impl TableViewItem<TorrentColumn> for Foo {
    fn to_column(&self, column: TorrentColumn) -> String {
        match column {
            TorrentColumn::Name => self.name.to_string(),
            TorrentColumn::Status => format!("{}", self.status),
            TorrentColumn::Progress => format!("{}", self.progress),
            TorrentColumn::Up => format!("{}", self.up),
            TorrentColumn::Down => format!("{}", self.down),
            TorrentColumn::NPeers => format!("{}", self.npeers),
        }
    }

    fn cmp(&self, other: &Self, column: TorrentColumn) -> Ordering
    where
        Self: Sized,
    {
        match column {
            TorrentColumn::Name => self.name.cmp(&other.name),
            TorrentColumn::Status => self.status.cmp(&other.status),
            TorrentColumn::Progress => self.progress.cmp(&other.progress),
            TorrentColumn::Up => self.up.cmp(&other.up),
            TorrentColumn::Down => self.down.cmp(&other.down),
            TorrentColumn::NPeers => self.npeers.cmp(&other.npeers),
        }
    }
}

pub fn new() -> BoxView<IdView<TableView<Foo, TorrentColumn>>> {
    let mut table =
        TableView::<Foo, TorrentColumn>::new()
            .column(TorrentColumn::Name, "Name", |c| c.width_percent(100 / 6))
            .column(TorrentColumn::Status, "Count", |c| c.width_percent(100 / 6))
            .column(TorrentColumn::Progress, "Progress", |c| {
                c.width_percent(100 / 6)
            })
            .column(TorrentColumn::Up, "Up", |c| c.width_percent(100 / 6))
            .column(TorrentColumn::Down, "Down", |c| c.width_percent(100 / 6))
            .column(TorrentColumn::NPeers, "#Peers", |c| {
                c.width_percent(100 / 6)
            });

    let mut items = Vec::new();
    for i in 0..50 {
        items.push(Foo {
            name: format!("Name {}", i),
            status: i,
            progress: i,
            up: i,
            down: i,
            npeers: i,
        });
    }

    table.set_items(items);

    table.set_on_sort(
        |siv: &mut Cursive, column: TorrentColumn, order: Ordering| {
            siv.add_layer(
                Dialog::around(TextView::new(format!("{} / {:?}", column.as_str(), order)))
                    .title("Sorted by")
                    .button("Close", |s| s.pop_layer()),
            );
        },
    );

    table.set_on_submit(|siv: &mut Cursive, row: usize, index: usize| {
        let value = siv.call_on_id("table", move |table: &mut TableView<Foo, TorrentColumn>| {
            format!("{:?}", table.borrow_item(index).unwrap())
        }).unwrap();

        siv.add_layer(
            Dialog::around(TextView::new(value))
                .title(format!("Removing row # {}", row))
                .button("Close", move |s| {
                    s.call_on_id("table", |table: &mut TableView<Foo, TorrentColumn>| {
                        table.remove_item(index);
                    });
                    s.pop_layer()
                }),
        );
    });

    return table.with_id("table").min_size((50, 20));
}
