/// A complete trading matching engine: can be used for e.g. simulations, or for implementing
/// an exchange.

mod arena;
mod test;

use std::collections::{BTreeMap, Bound};
use self::arena::{Index, Arena};
use std::{mem, fmt};
use crate::*;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
/// Side of an order.
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
/// An order.
pub struct Order {
    /// Order price.
    pub price: Price,

    /// Order size, in atomic asset units.
    pub size: usize,

    /// Order side: `Buy` or `Sell`.
    pub side: Side,

    /// ID of the order owner.
    pub owner: TraderId,
}

/// An identifier which should uniquely determine an entry.
pub type EntryId = usize;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
/// A limit order at some price limit of the order book.
struct BookEntry {
    /// Size of the limit order.
    size: usize,

    /// Pointer to the next order at this price limit. If `None`, then this entry
    /// is the last one at this price limit.
    next: Option<Index>,

    id: EntryId,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
/// Pointers to begin and end of the book entries list.
struct Link {
    head: Index,
    tail: Index,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
/// A price limit in the order book.
struct PriceLimit {
    /// If `link` is `None`, the limit is empty. Else, it gives a
    /// link to the book entries list for that limit.
    link: Option<Link>,
}

type PriceLimits = BTreeMap<Price, PriceLimit>;
type BookEntries = Arena<BookEntry>;

#[derive(Clone, Debug)]
/// A matching engine.
pub struct MatchingEngine {
    /// The various price limits, which are initialized lazily.
    price_limits: PriceLimits,

    /// A memory arena for storing book entries, independently of their actual price limit.
    entries: BookEntries,

    /// INVARIANT: best limits are *NEVER* empty, unless their value is `0` or
    /// `Price::max_value()`. Moreover, the price range `(best_bid, best_ask)` is
    /// *EMPTY*.
    best_bid: Price,
    best_ask: Price,

    max_entry_id: EntryId,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum ExecResult {
    Filled(Order),
    NotExecuted,
}

trait Executor {
    fn exec(&mut self, link: &Link, order: Order) -> (Option<Index>, Order);

    fn exec_range<'a, I: Iterator<Item = (&'a Price, &'a mut PriceLimit)>>(
        &mut self,
        order: Order,
        range: I
    ) -> (Price, ExecResult);
}

impl Executor for BookEntries {
    /// Make an order cross through a price limit. Return the updated order (which accounts for
    /// how much the order was filled), as well as an `Index` which points to the first entry
    /// at this price limit which was not exhausted, if any.
    fn exec(&mut self, link: &Link, mut order: Order) -> (Option<Index>, Order) {
        let mut maybe_index = Some(link.head);
        while let Some(index) = maybe_index {
            {
                let entry = self.get_mut(index);
                if entry.size <= order.size {
                    // This entry is exhausted by the incoming order.
                    order.size -= entry.size;
                    entry.size = 0;
                    maybe_index = entry.next;
                } else {
                    // The order has been completely filled.
                    entry.size -= order.size;
                    order.size = 0;
                    break;
                }
            }
            // If we are here, then the entry referenced by `index` has been exhausted.
            // We free it from the arena.
            self.free(index);
        }
        (maybe_index, order)
    }

    /// Make an order cross through a range of price limits. Return a `Price` corresponding to
    /// the first non exhausted limit (if it makes sense), along with an `ExecResult`:
    /// * `ExecResult::Filled(updated_order)` if the order was (partially) filled, with
    ///   `updated_order` accounting for how much the order was filled
    ///   updated depending on the side of the order.
    /// * `ExecResult::NotExecuted` if the range was empty.
    fn exec_range<'a, I: Iterator<Item = (&'a Price, &'a mut PriceLimit)>>(
        &mut self,
        mut order: Order,
        range: I
    ) -> (Price, ExecResult)
    {
        let mut exec_result = ExecResult::NotExecuted;
        for (price, limit) in range {
            if let Some(ref link) = limit.link {
                let (maybe_index, new_order) = self.exec(link, order.clone());
                order = new_order;
                exec_result = ExecResult::Filled(order.clone());

                match maybe_index {
                    // All the indices prior to `index` were exhausted, hence we update the
                    // beginning of the entries list. Also we are sure that the order was
                    // completely filled, we can return.
                    Some(index) => {
                        limit.link.as_mut().unwrap().head = index;
                        return (*price, exec_result);
                    }

                    // All the entries at this price limit were exhausted, hence we mark
                    // this price limit as empty.
                    None => limit.link = None,
                }
            }
        }
        match order.side {
            Side::Buy => (order.price + 1, exec_result),
            Side::Sell => (order.price - 1, exec_result),
        }
    }
}

impl MatchingEngine {
    /// Create a new matchin engine, pre-allocating `capacity` book entries.
    pub fn new(capacity: usize) -> Self {
        MatchingEngine {
            price_limits: PriceLimits::new(),
            entries: BookEntries::new(capacity),
            best_bid: 0,
            best_ask: Price::max_value(),
            max_entry_id: 0,
        }
    }

    /// Return the best prices, respectively best bid and best ask.
    pub fn best_limits(&self) -> (Price, Price) {
        (self.best_bid, self.best_ask)
    }

    pub fn size_at_price(&self, price: Price) -> usize {
        if let Some(limit) = self.price_limits.get(&price) {
            return self.size_at_limit(limit);
        }
        0
    }

    /// Compute the total size of a given limit.
    fn size_at_limit(&self, limit: &PriceLimit) -> usize {
        match limit.link {
            Some(ref link) => {
                let mut count = 0;
                let mut maybe_index = Some(link.head);
                while let Some(index) = maybe_index {
                    let entry = self.entries.get(index);
                    count += entry.size;
                    maybe_index = entry.next;
                }
                count
            },
            None => 0,
        }
    }

    /// Insert an order in the order book, and update best limits consequently.
    fn insert_order(&mut self, order: Order) -> EntryId {
        let id = self.max_entry_id;
        let index = self.entries.alloc(BookEntry {
            size: order.size,
            next: None,
            id,
        });

        self.max_entry_id += 1;

        let price_point =
            self.price_limits
                .entry(order.price)
                .or_insert_with(|| PriceLimit { link: None });

        if price_point.link.is_some() {
            let link = price_point.link.as_mut().unwrap();
            self.entries.get_mut(link.tail).next = Some(index);
                link.tail = index;
        } else {
            mem::replace(&mut price_point.link, Some(Link {
                head: index,
                tail: index,
            }));
        }

        match order.side {
            Side::Buy if order.price > self.best_bid => {
                self.best_bid = order.price;
            },
            Side::Sell if order.price < self.best_ask => {
                self.best_ask = order.price;
            },
            _ => (),
        }

        id
    }

    /// Match or insert a limit order. If the order was inserted in the order book, return the
    /// corresponding `EntryId`.
    pub fn limit(&mut self, order: Order) -> Option<EntryId> {
        let (new_price, exec_result) = match order.side {
            Side::Buy if order.price >= self.best_ask => {
                let range = self.price_limits.range_mut(
                    (Bound::Included(self.best_ask), Bound::Included(order.price))
                );
                self.entries.exec_range(order.clone(), range)
            },
            Side::Sell if order.price <= self.best_bid => {
                let range = self.price_limits.range_mut(
                    (Bound::Included(order.price), Bound::Included(self.best_bid))
                ).rev();
                self.entries.exec_range(order.clone(), range)
            },
            _ => (0, ExecResult::NotExecuted)
        };

        match exec_result {
            // The previous range was empty, i.e. the limit order is not marketable and should
            // be inserted in the order book.
            ExecResult::NotExecuted => {
                Some(self.insert_order(order))
            },
            ExecResult::Filled(updated_order) => {
                // Go find the new best limit.
                match order.side {
                    Side::Buy => {
                        let maybe_best_ask = self.price_limits.range_mut(
                            (Bound::Included(new_price), Bound::Included(Price::max_value()))
                        ).find(|(_, limit)| limit.link.is_some());

                        match maybe_best_ask {
                            Some((best_price, _)) => self.best_ask = *best_price,
                            None => self.best_ask = Price::max_value(),
                        }
                    },
                    Side::Sell => {
                        let maybe_best_bid = self.price_limits.range_mut(
                            (Bound::Included(0), Bound::Included(new_price))
                        ).rev().find(|(_, limit)| limit.link.is_some());

                        match maybe_best_bid {
                            Some((best_price, _)) => self.best_bid = *best_price,
                            None => self.best_bid = 0,
                        }
                    }
                };

                // The order has exhausted the whole range, we insert what remains.
                if updated_order.size > 0 {
                    Some(self.insert_order(updated_order))
                } else {
                    None
                }
            }
        }
    }
}

impl fmt::Display for MatchingEngine {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut bid = true;
        write!(f, "--- ASK ---\n")?;
        for (price, limit) in self.price_limits.iter().rev() {
            if bid && *price < self.best_ask {
                write!(f, "--- BID ---\n")?;
                bid = false;
            }
            let size = self.size_at_limit(limit);
            if size > 0 {
                write!(f, "{}: {}\n", price, self.size_at_limit(limit))?;
            }
        }
        if bid {
            write!(f, "--- BID ---\n")?;
        }
        Ok(())
    }
}
