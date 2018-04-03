use std::hash::Hash;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use cursive::{Cursive, Printer};
use cursive::vec::Vec2;
use cursive::traits::*;
use cursive::views::{BoxView, IdView};
use cursive::traits::*;
use cursive::align::HAlign;
use cursive::views::{Dialog, TextView};

use cursive_table_view::{TableColumn, TableView, TableViewItem};
use bittorrent::bittorrent::manager::{Info, InfoMsg, Manager, Status};
use bittorrent::bittorrent::Hash as TorrentHash;

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

// too much boilerplate here, but hard to do this by macro
impl TableViewItem<TorrentColumn> for Info {
    fn to_column(&self, column: TorrentColumn) -> String {
        match column {
            TorrentColumn::Name => self.name.to_string(),
            TorrentColumn::Status => format!("{:?}", self.status),
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
            TorrentColumn::Progress => {
                format!("{}", self.progress).cmp(&format!("{}", other.progress))
            }
            TorrentColumn::Up => self.up.cmp(&other.up),
            TorrentColumn::Down => self.down.cmp(&other.down),
            TorrentColumn::NPeers => self.npeers.cmp(&other.npeers),
        }
    }
}

pub struct AsyncTableView<T: TableViewItem<H> + 'static, H: Eq + Hash + Copy + Clone + 'static> {
    rx: Receiver<InfoMsg>,
    table: TableView<T, H>,
    index_map: HashMap<TorrentHash, usize>,
}

impl<T: TableViewItem<H> + 'static, H: Eq + Hash + Copy + Clone + 'static> AsyncTableView<T, H> {
    pub fn new(rx: Receiver<InfoMsg>, table: TableView<T, H>) -> AsyncTableView<T, H> {
        AsyncTableView {
            rx: rx,
            table: table,
            index_map: HashMap::new(),
        }
    }
}

impl View for AsyncTableView<Info, TorrentColumn> {
    fn layout(&mut self, _: Vec2) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                InfoMsg::All(vec) => {
                    self.index_map = vec.iter()
                        .enumerate()
                        .map(|(i, info)| (info.info_hash.clone(), i))
                        .collect();
                    self.table.set_items(vec);
                }
                InfoMsg::One(info) => match info.status {
                    Status::Complete => {
                        let i = {
                            || {
                                *self.index_map
                                    .get(&info.info_hash)
                                    .ok_or(())
                                    .expect("Expected info_hash to be in index_map")
                            }
                        }();
                        self.table.remove_item(i).or_else(|| {
                            error!(
                                "Attempted to remove item from table that wasn't present: {:?}",
                                info
                            );
                            None
                        });
                        self.index_map.remove(&info.info_hash);
                    }
                    _ => {
                        let len = self.index_map.len();
                        self.index_map.insert(info.info_hash, len);
                        self.table.insert_item(info);
                    }
                },
            }
        }
    }

    fn draw(&self, printer: &Printer) {
        self.table.draw(printer);
    }
}

pub fn new() -> TableView<Info, TorrentColumn> {
    let mut table =
        TableView::<Info, TorrentColumn>::new()
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

    /*
    let mut items = Vec::new();
    for i in 0..50 {
        items.push(Info {
            name: format!("Name {}", i),
            status: i,
            progress: i as f32,
            up: i,
            down: i,
            npeers: i,
        });
    }

    table.set_items(items);*/

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
        let value = siv.call_on_id(
            "table",
            move |table: &mut TableView<Info, TorrentColumn>| {
                format!("{:?}", table.borrow_item(index).unwrap())
            },
        ).unwrap();

        siv.add_layer(
            Dialog::around(TextView::new(value))
                .title(format!("Removing row # {}", row))
                .button("Close", move |s| {
                    s.call_on_id("table", |table: &mut TableView<Info, TorrentColumn>| {
                        table.remove_item(index);
                    });
                    s.pop_layer()
                }),
        );
    });

    return table;
}
