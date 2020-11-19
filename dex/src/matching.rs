use std::num::NonZeroU64;

use crate::instruction::SelfTradeBehavior;
use num_enum::{IntoPrimitive, TryFromPrimitive};
#[cfg(test)]
use proptest_derive::Arbitrary;
use serde::{Deserialize, Serialize};
#[cfg(feature = "program")]
use solana_sdk::info;

use crate::critbit::SlabTreeError;
use crate::error::{DexErrorCode, DexResult, SourceFileId};
use crate::{
    critbit::{LeafNode, NodeHandle, Slab, SlabView},
    error::DexError,
    fees::{self, FeeTier},
    state::{Event, EventQueue, EventView, MarketState, Request, RequestQueue, RequestView},
};

#[cfg(not(feature = "program"))]
macro_rules! info {
    ($($i:expr),*) => { { ($($i),*) } };
}
declare_check_assert_macros!(SourceFileId::Matching);

#[derive(
    Eq, PartialEq, Copy, Clone, TryFromPrimitive, IntoPrimitive, Debug, Serialize, Deserialize,
)]
#[cfg_attr(test, derive(Arbitrary))]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
#[repr(u8)]
pub enum Side {
    Bid = 0,
    Ask = 1,
}

#[derive(
    Eq, PartialEq, Copy, Clone, TryFromPrimitive, IntoPrimitive, Debug, Serialize, Deserialize,
)]
#[cfg_attr(test, derive(Arbitrary))]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
#[repr(u8)]
pub enum OrderType {
    Limit = 0,
    ImmediateOrCancel = 1,
    PostOnly = 2,
}

fn extract_price_from_order_id(order_id: &u128) -> u64 {
    (order_id >> 64) as u64
}

pub struct OrderBookState<'a> {
    // first byte of a key is 0xaa or 0xbb, disambiguating bids and asks
    pub bids: &'a mut Slab,
    pub asks: &'a mut Slab,
    pub market_state: &'a mut MarketState,
}

impl<'ob> OrderBookState<'ob> {
    pub fn orders_mut(&mut self, side: Side) -> &mut Slab {
        match side {
            Side::Bid => self.bids,
            Side::Ask => self.asks,
        }
    }

    pub fn find_bbo(&self, side: Side) -> Option<NodeHandle> {
        match side {
            Side::Bid => self.bids.find_max(),
            Side::Ask => self.asks.find_min(),
        }
    }

    pub fn process_requests(
        &mut self,
        req_q: &mut RequestQueue,
        event_q: &mut EventQueue,
        limit: u16,
    ) -> Result<(), DexError> {
        let mut limit_remaining = limit;
        while limit_remaining > 0 {
            let request = match req_q.peek_front_mut() {
                Some(r) => r,
                None => break,
            };
            match self.process_orderbook_request(request, event_q, &mut limit_remaining)? {
                Some(remaining_request) => {
                    *request = remaining_request;
                }
                None => {
                    req_q.pop_front().unwrap();
                }
            };
        }

        Ok(())
    }

    fn process_orderbook_request(
        &mut self,
        request: &Request,
        event_q: &mut EventQueue,
        limit: &mut u16,
    ) -> DexResult<Option<Request>> {
        // println!("{:#?}", request.as_view()?);
        Ok(match request.as_view()? {
            RequestView::NewOrder {
                side,
                order_type,
                order_id,
                owner_slot,
                fee_tier,
                owner,
                max_coin_qty,
                native_pc_qty_locked,
                client_order_id,
                self_trade_behavior,
            } => self
                .new_order(
                    NewOrderParams {
                        side,
                        order_type,
                        order_id,
                        owner,
                        owner_slot,
                        fee_tier,
                        max_coin_qty,
                        native_pc_qty_locked,
                        client_order_id: client_order_id.map_or(0, NonZeroU64::get),
                        self_trade_behavior,
                    },
                    event_q,
                    limit,
                )?
                .map(|remaining| {
                    Request::new(RequestView::NewOrder {
                        side,
                        order_type,
                        order_id,
                        owner_slot,
                        fee_tier,
                        owner,
                        max_coin_qty: remaining.coin_qty_remaining,
                        native_pc_qty_locked: remaining.native_pc_qty_remaining,
                        client_order_id,
                        self_trade_behavior,
                    })
                }),
            RequestView::CancelOrder {
                side,
                order_id,
                expected_owner_slot,
                expected_owner,
                client_order_id,
                cancel_id: _,
            } => {
                *limit -= 1;
                self.cancel_order(
                    side,
                    order_id,
                    expected_owner,
                    expected_owner_slot,
                    client_order_id,
                    event_q,
                )?;
                None
            }
        })
    }
}

struct NewOrderParams<'a> {
    side: Side,
    order_type: OrderType,
    order_id: &'a u128,
    owner: &'a [u64; 4],
    owner_slot: u8,
    fee_tier: FeeTier,
    max_coin_qty: NonZeroU64,
    native_pc_qty_locked: Option<NonZeroU64>,
    client_order_id: u64,
    self_trade_behavior: SelfTradeBehavior,
}

struct OrderRemaining {
    coin_qty_remaining: NonZeroU64,
    native_pc_qty_remaining: Option<NonZeroU64>,
}

impl<'ob> OrderBookState<'ob> {
    fn new_order(
        &mut self,

        params: NewOrderParams,

        event_q: &mut EventQueue,
        limit: &mut u16,
    ) -> DexResult<Option<OrderRemaining>> {
        let NewOrderParams {
            side,
            order_type,
            order_id,
            owner,
            owner_slot,
            fee_tier,
            mut max_coin_qty,
            mut native_pc_qty_locked,
            client_order_id,
            self_trade_behavior,
        } = params;
        let (post_only, post_allowed) = match order_type {
            OrderType::Limit => (false, true),
            OrderType::ImmediateOrCancel => (false, false),
            OrderType::PostOnly => (true, true),
        };
        let limit_price = extract_price_from_order_id(order_id);
        while *limit > 0 {
            *limit -= 1;
            let remaining_order = match side {
                Side::Bid => self.new_bid(
                    NewBidParams {
                        max_coin_qty,
                        native_pc_qty_locked: native_pc_qty_locked.unwrap(),
                        limit_price: NonZeroU64::new(limit_price),
                        order_id,
                        owner,
                        owner_slot,
                        fee_tier,
                        post_only,
                        post_allowed,
                        client_order_id,
                        self_trade_behavior,
                    },
                    event_q,
                ),
                Side::Ask => {
                    native_pc_qty_locked.ok_or(()).unwrap_err();
                    self.new_ask(
                        NewAskParams {
                            max_qty: max_coin_qty,
                            limit_price: NonZeroU64::new(limit_price).unwrap(),
                            order_id,
                            owner,
                            owner_slot,
                            fee_tier,
                            post_only,
                            post_allowed,
                            client_order_id,
                            self_trade_behavior,
                        },
                        event_q,
                    )
                }
            }?;
            if *limit == 0 {
                return Ok(remaining_order);
            }
            match remaining_order {
                Some(remaining_order) => {
                    max_coin_qty = remaining_order.coin_qty_remaining;
                    native_pc_qty_locked = remaining_order.native_pc_qty_remaining;
                }
                None => break,
            };
        }
        Ok(None)
    }
}

struct NewAskParams<'a> {
    max_qty: NonZeroU64,
    limit_price: NonZeroU64,
    order_id: &'a u128,
    owner: &'a [u64; 4],
    owner_slot: u8,
    fee_tier: FeeTier,
    post_only: bool,
    post_allowed: bool,
    client_order_id: u64,
    self_trade_behavior: SelfTradeBehavior,
}

impl<'ob> OrderBookState<'ob> {
    fn new_ask(
        &mut self,
        params: NewAskParams,
        event_q: &mut EventQueue,
    ) -> DexResult<Option<OrderRemaining>> {
        let NewAskParams {
            max_qty,
            limit_price,
            order_id,
            owner,
            owner_slot,
            fee_tier,
            post_only,
            post_allowed,
            client_order_id,
            self_trade_behavior,
        } = params;
        let mut unfilled_qty = max_qty.get();
        let mut accum_fill_price = 0;

        let pc_lot_size = self.market_state.pc_lot_size;
        let coin_lot_size = self.market_state.coin_lot_size;

        let mut accum_maker_rebates = 0;
        let crossed;
        let done = loop {
            let best_bid_h = match self.find_bbo(Side::Bid) {
                None => {
                    crossed = false;
                    break true;
                }
                Some(h) => h,
            };

            let best_bid_ref = self
                .orders_mut(Side::Bid)
                .get_mut(best_bid_h)
                .unwrap()
                .as_leaf_mut()
                .unwrap();

            let trade_price = best_bid_ref.price();
            crossed = limit_price <= trade_price;

            if !crossed || post_only {
                break true;
            }

            let bid_size = best_bid_ref.quantity();
            let trade_qty = bid_size.min(unfilled_qty);

            if trade_qty == 0 {
                break true;
            }

            let order_would_self_trade = owner == best_bid_ref.owner();
            if order_would_self_trade {
                let best_bid_id = *best_bid_ref.order_id();
                let cancelled_provide_qty;
                let cancelled_take_qty;

                match self_trade_behavior {
                    SelfTradeBehavior::DecrementTake => {
                        cancelled_provide_qty = trade_qty;
                        cancelled_take_qty = trade_qty;
                    }
                    SelfTradeBehavior::CancelProvide => {
                        cancelled_provide_qty = best_bid_ref.quantity();
                        cancelled_take_qty = 0;
                    }
                };

                let remaining_provide_size = bid_size - cancelled_provide_qty;
                let provide_out = Event::new(EventView::Out {
                    side: Side::Bid,
                    native_qty_unlocked: cancelled_provide_qty * trade_price.get() * pc_lot_size,
                    native_qty_still_locked: remaining_provide_size
                        * trade_price.get()
                        * pc_lot_size,
                    order_id: &best_bid_id,
                    owner: best_bid_ref.owner(),
                    owner_slot: best_bid_ref.owner_slot(),
                    client_order_id: NonZeroU64::new(best_bid_ref.client_order_id()),
                });
                event_q
                    .push_back(provide_out)
                    .map_err(|_| DexErrorCode::EventQueueFull)?;
                if remaining_provide_size == 0 {
                    self.orders_mut(Side::Bid)
                        .remove_by_key(&best_bid_id)
                        .unwrap();
                } else {
                    *best_bid_ref.quantity_mut() = remaining_provide_size;
                }

                unfilled_qty -= cancelled_take_qty;
                let take_out = Event::new(EventView::Out {
                    side: Side::Ask,
                    native_qty_unlocked: cancelled_take_qty * coin_lot_size,
                    native_qty_still_locked: unfilled_qty,
                    order_id,
                    owner,
                    owner_slot,
                    client_order_id: NonZeroU64::new(client_order_id),
                });
                event_q
                    .push_back(take_out)
                    .map_err(|_| DexErrorCode::EventQueueFull)?;

                let order_remaining =
                    NonZeroU64::new(unfilled_qty).map(|coin_qty_remaining| OrderRemaining {
                        coin_qty_remaining,
                        native_pc_qty_remaining: None,
                    });
                return Ok(order_remaining);
            }

            let maker_fee_tier = best_bid_ref.fee_tier();
            let native_maker_pc_qty = trade_qty * trade_price.get() * pc_lot_size;
            let native_maker_rebate = maker_fee_tier.maker_rebate(native_maker_pc_qty);
            accum_maker_rebates += native_maker_rebate;

            let maker_fill = Event::new(EventView::Fill {
                side: Side::Bid,
                maker: true,
                native_qty_paid: native_maker_pc_qty - native_maker_rebate,
                native_qty_received: trade_qty * coin_lot_size,
                native_fee_or_rebate: native_maker_rebate,
                order_id: best_bid_ref.order_id(),
                owner: best_bid_ref.owner(),
                owner_slot: best_bid_ref.owner_slot(),
                fee_tier: maker_fee_tier,
                client_order_id: NonZeroU64::new(best_bid_ref.client_order_id()),
            });
            event_q
                .push_back(maker_fill)
                .map_err(|_| DexErrorCode::EventQueueFull)?;

            *best_bid_ref.quantity_mut() -= trade_qty;
            unfilled_qty -= trade_qty;
            accum_fill_price += trade_qty * trade_price.get();

            if best_bid_ref.quantity() == 0 {
                let best_bid_id = *best_bid_ref.order_id();
                event_q
                    .push_back(Event::new(EventView::Out {
                        side: Side::Bid,
                        native_qty_unlocked: 0,
                        native_qty_still_locked: 0,
                        order_id: &best_bid_id,
                        owner: best_bid_ref.owner(),
                        owner_slot: best_bid_ref.owner_slot(),
                        client_order_id: NonZeroU64::new(best_bid_ref.client_order_id()),
                    }))
                    .map_err(|_| DexErrorCode::EventQueueFull)?;
                self.orders_mut(Side::Bid)
                    .remove_by_key(&best_bid_id)
                    .unwrap();
            }

            break false;
        };

        let native_taker_pc_qty = accum_fill_price * pc_lot_size;
        let native_taker_fee = fee_tier.taker_fee(native_taker_pc_qty);
        if native_taker_pc_qty > 0 {
            let taker_fill = Event::new(EventView::Fill {
                side: Side::Ask,
                maker: false,
                native_qty_paid: (max_qty.get() - unfilled_qty) * coin_lot_size,
                native_qty_received: native_taker_pc_qty - native_taker_fee,
                native_fee_or_rebate: native_taker_fee,
                order_id,
                owner,
                owner_slot,
                fee_tier,
                client_order_id: NonZeroU64::new(client_order_id),
            });
            event_q
                .push_back(taker_fill)
                .map_err(|_| DexErrorCode::EventQueueFull)?;
        }

        let net_fees_before_referrer_rebate = native_taker_fee - accum_maker_rebates;
        let referrer_rebate = fees::referrer_rebate(native_taker_fee);
        let net_fees = net_fees_before_referrer_rebate - referrer_rebate;

        self.market_state.referrer_rebates_accrued += referrer_rebate;
        self.market_state.pc_fees_accrued += net_fees;
        self.market_state.pc_deposits_total -= net_fees_before_referrer_rebate;

        if !done {
            if let Some(coin_qty_remaining) = NonZeroU64::new(unfilled_qty) {
                return Ok(Some(OrderRemaining {
                    coin_qty_remaining,
                    native_pc_qty_remaining: None,
                }));
            }
        }

        if post_allowed && !crossed && unfilled_qty > 0 {
            let offers = self.orders_mut(Side::Ask);
            let new_order = LeafNode::new(
                owner_slot,
                order_id,
                owner,
                unfilled_qty,
                fee_tier,
                client_order_id,
            );
            let insert_result = offers.insert_leaf(&new_order);
            if let Err(SlabTreeError::OutOfSpace) = insert_result {
                // boot out the least aggressive offer
                info!("offers full! booting...");
                let order = offers.remove_max().unwrap();
                let out = Event::new(EventView::Out {
                    side: Side::Ask,
                    native_qty_unlocked: order.quantity() * coin_lot_size,
                    native_qty_still_locked: 0,
                    order_id: order.order_id(),
                    owner: order.owner(),
                    owner_slot: order.owner_slot(),
                    client_order_id: NonZeroU64::new(order.client_order_id()),
                });
                event_q
                    .push_back(out)
                    .map_err(|_| DexErrorCode::EventQueueFull)?;
                offers.insert_leaf(&new_order).unwrap();
            } else {
                insert_result.unwrap();
            }
        } else {
            let out = Event::new(EventView::Out {
                side: Side::Ask,
                native_qty_unlocked: unfilled_qty * coin_lot_size,
                native_qty_still_locked: 0,
                order_id,
                owner,
                owner_slot,
                client_order_id: NonZeroU64::new(client_order_id),
            });
            event_q
                .push_back(out)
                .map_err(|_| DexErrorCode::EventQueueFull)?;
        }

        Ok(None)
    }
}

struct NewBidParams<'a> {
    max_coin_qty: NonZeroU64,
    native_pc_qty_locked: NonZeroU64,
    limit_price: Option<NonZeroU64>,
    order_id: &'a u128,
    owner: &'a [u64; 4],
    owner_slot: u8,
    fee_tier: FeeTier,
    post_only: bool,
    post_allowed: bool,
    client_order_id: u64,
    self_trade_behavior: SelfTradeBehavior,
}

impl<'ob> OrderBookState<'ob> {
    fn new_bid(
        &mut self,
        params: NewBidParams,
        event_q: &mut EventQueue,
    ) -> DexResult<Option<OrderRemaining>> {
        let NewBidParams {
            max_coin_qty,
            native_pc_qty_locked,
            limit_price,
            order_id,
            owner,
            owner_slot,
            fee_tier,
            post_only,
            post_allowed,
            client_order_id,
            self_trade_behavior,
        } = params;
        if post_allowed {
            check_assert!(limit_price.is_some())?;
        }

        let pc_lot_size = self.market_state.pc_lot_size;
        let coin_lot_size = self.market_state.coin_lot_size;

        let max_pc_qty = fee_tier.remove_taker_fee(native_pc_qty_locked.get()) / pc_lot_size;

        let mut coin_qty_remaining = max_coin_qty.get();
        let mut pc_qty_remaining = max_pc_qty;
        let mut accum_maker_rebates = 0;

        let crossed;
        let done = loop {
            let best_offer_h = match self.find_bbo(Side::Ask) {
                None => {
                    crossed = false;
                    break true;
                }
                Some(h) => h,
            };

            let best_offer_ref = self
                .orders_mut(Side::Ask)
                .get_mut(best_offer_h)
                .unwrap()
                .as_leaf_mut()
                .unwrap();

            let trade_price = best_offer_ref.price();
            crossed = limit_price
                .map(|limit_price| limit_price >= trade_price)
                .unwrap_or(true);
            if !crossed || post_only {
                break true;
            }

            let offer_size = best_offer_ref.quantity();
            let trade_qty = offer_size
                .min(coin_qty_remaining)
                .min(pc_qty_remaining / best_offer_ref.price().get());

            if trade_qty == 0 {
                break true;
            }

            let order_would_self_trade = owner == best_offer_ref.owner();
            if order_would_self_trade {
                let best_offer_id = *best_offer_ref.order_id();

                let cancelled_take_qty;
                let cancelled_provide_qty;

                match self_trade_behavior {
                    SelfTradeBehavior::CancelProvide => {
                        cancelled_take_qty = 0;
                        cancelled_provide_qty = best_offer_ref.quantity();
                    }
                    SelfTradeBehavior::DecrementTake => {
                        cancelled_take_qty = trade_qty;
                        cancelled_provide_qty = trade_qty;
                    }
                };

                let remaining_provide_qty = best_offer_ref.quantity() - cancelled_provide_qty;
                let provide_out = Event::new(EventView::Out {
                    side: Side::Ask,
                    native_qty_unlocked: cancelled_provide_qty * coin_lot_size,
                    native_qty_still_locked: remaining_provide_qty * coin_lot_size,
                    order_id: &best_offer_id,
                    owner: best_offer_ref.owner(),
                    owner_slot: best_offer_ref.owner_slot(),
                    client_order_id: NonZeroU64::new(best_offer_ref.client_order_id()),
                });
                event_q
                    .push_back(provide_out)
                    .map_err(|_| DexErrorCode::EventQueueFull)?;
                if remaining_provide_qty == 0 {
                    self.orders_mut(Side::Ask)
                        .remove_by_key(&best_offer_id)
                        .unwrap();
                } else {
                    *best_offer_ref.quantity_mut() = remaining_provide_qty;
                }

                let native_taker_pc_unlocked = cancelled_take_qty * trade_price.get() * pc_lot_size;
                let native_taker_pc_still_locked =
                    native_pc_qty_locked.get() - native_taker_pc_unlocked;

                let order_remaining = (|| {
                    Some(OrderRemaining {
                        coin_qty_remaining: NonZeroU64::new(
                            coin_qty_remaining - cancelled_take_qty,
                        )?,
                        native_pc_qty_remaining: Some(NonZeroU64::new(
                            native_taker_pc_still_locked,
                        )?),
                    })
                })();

                let take_out = {
                    let native_qty_unlocked;
                    let native_qty_still_locked;
                    match order_remaining {
                        Some(_) => {
                            native_qty_unlocked = native_taker_pc_unlocked;
                            native_qty_still_locked = native_taker_pc_still_locked;
                        }
                        None => {
                            native_qty_unlocked = native_pc_qty_locked.get();
                            native_qty_still_locked = 0;
                        }
                    };
                    Event::new(EventView::Out {
                        side: Side::Bid,
                        native_qty_unlocked,
                        native_qty_still_locked,
                        order_id,
                        owner,
                        owner_slot,
                        client_order_id: NonZeroU64::new(client_order_id),
                    })
                };
                event_q
                    .push_back(take_out)
                    .map_err(|_| DexErrorCode::EventQueueFull)?;

                return Ok(order_remaining);
            }
            let maker_fee_tier = best_offer_ref.fee_tier();
            let native_maker_pc_qty = trade_qty * trade_price.get() * pc_lot_size;
            let native_maker_rebate = maker_fee_tier.maker_rebate(native_maker_pc_qty);
            accum_maker_rebates += native_maker_rebate;

            let maker_fill = Event::new(EventView::Fill {
                side: Side::Ask,
                maker: true,
                native_qty_paid: trade_qty * coin_lot_size,
                native_qty_received: native_maker_pc_qty + native_maker_rebate,
                native_fee_or_rebate: native_maker_rebate,
                order_id: best_offer_ref.order_id(),
                owner: best_offer_ref.owner(),
                owner_slot: best_offer_ref.owner_slot(),
                fee_tier: maker_fee_tier,
                client_order_id: NonZeroU64::new(best_offer_ref.client_order_id()),
            });
            event_q
                .push_back(maker_fill)
                .map_err(|_| DexErrorCode::EventQueueFull)?;

            *best_offer_ref.quantity_mut() -= trade_qty;
            coin_qty_remaining -= trade_qty;
            pc_qty_remaining -= trade_qty * trade_price.get();

            if best_offer_ref.quantity() == 0 {
                let best_offer_id = *best_offer_ref.order_id();
                event_q
                    .push_back(Event::new(EventView::Out {
                        side: Side::Ask,
                        native_qty_unlocked: 0,
                        native_qty_still_locked: 0,
                        order_id: &best_offer_id,
                        owner: best_offer_ref.owner(),
                        owner_slot: best_offer_ref.owner_slot(),
                        client_order_id: NonZeroU64::new(best_offer_ref.client_order_id()),
                    }))
                    .map_err(|_| DexErrorCode::EventQueueFull)?;
                self.orders_mut(Side::Ask)
                    .remove_by_key(&best_offer_id)
                    .unwrap();
            }

            break false;
        };

        let native_accum_fill_price = (max_pc_qty - pc_qty_remaining) * pc_lot_size;
        let native_taker_fee = fee_tier.taker_fee(native_accum_fill_price);
        let native_pc_qty_remaining =
            native_pc_qty_locked.get() - native_accum_fill_price - native_taker_fee;

        if native_accum_fill_price > 0 {
            let taker_fill = Event::new(EventView::Fill {
                side: Side::Bid,
                maker: false,
                native_qty_paid: native_accum_fill_price + native_taker_fee,
                native_qty_received: (max_coin_qty.get() - coin_qty_remaining) * coin_lot_size,
                native_fee_or_rebate: native_taker_fee,
                order_id,
                owner,
                owner_slot,
                fee_tier,
                client_order_id: NonZeroU64::new(client_order_id),
            });
            event_q
                .push_back(taker_fill)
                .map_err(|_| DexErrorCode::EventQueueFull)?;
        }

        let net_fees_before_referrer_rebate = native_taker_fee - accum_maker_rebates;
        let referrer_rebate = fees::referrer_rebate(native_taker_fee);
        let net_fees = net_fees_before_referrer_rebate - referrer_rebate;

        self.market_state.referrer_rebates_accrued += referrer_rebate;
        self.market_state.pc_fees_accrued += net_fees;
        self.market_state.pc_deposits_total -= net_fees_before_referrer_rebate;

        if !done {
            if let Some(coin_qty_remaining) = NonZeroU64::new(coin_qty_remaining) {
                if let Some(native_pc_qty_remaining) = NonZeroU64::new(native_pc_qty_remaining) {
                    return Ok(Some(OrderRemaining {
                        coin_qty_remaining,
                        native_pc_qty_remaining: Some(native_pc_qty_remaining),
                    }));
                }
            }
        }

        let (coin_qty_to_post, pc_qty_to_keep_locked) = match limit_price {
            Some(price) if post_allowed && !crossed => {
                let coin_qty_to_post =
                    coin_qty_remaining.min(native_pc_qty_remaining / pc_lot_size / price.get());
                (coin_qty_to_post, coin_qty_to_post * price.get())
            }
            _ => (0, 0),
        };

        let out = {
            let native_qty_still_locked = pc_qty_to_keep_locked * pc_lot_size;
            let native_qty_unlocked = native_pc_qty_remaining - native_qty_still_locked;
            Event::new(EventView::Out {
                side: Side::Bid,
                native_qty_unlocked,
                native_qty_still_locked,
                order_id,
                owner,
                owner_slot,
                client_order_id: NonZeroU64::new(client_order_id),
            })
        };
        event_q
            .push_back(out)
            .map_err(|_| DexErrorCode::EventQueueFull)?;

        if pc_qty_to_keep_locked > 0 {
            let bids = self.orders_mut(Side::Bid);
            let new_leaf = LeafNode::new(
                owner_slot,
                order_id,
                owner,
                coin_qty_to_post,
                fee_tier,
                client_order_id,
            );
            let insert_result = bids.insert_leaf(&new_leaf);
            if let Err(SlabTreeError::OutOfSpace) = insert_result {
                // boot out the least aggressive bid
                info!("bids full! booting...");
                let order = bids.remove_min().unwrap();
                let out = Event::new(EventView::Out {
                    side: Side::Bid,
                    native_qty_unlocked: order.quantity() * order.price().get() * pc_lot_size,
                    native_qty_still_locked: 0,
                    order_id: order.order_id(),
                    owner: order.owner(),
                    owner_slot: order.owner_slot(),
                    client_order_id: NonZeroU64::new(order.client_order_id()),
                });
                event_q
                    .push_back(out)
                    .map_err(|_| DexErrorCode::EventQueueFull)?;
                bids.insert_leaf(&new_leaf).unwrap();
            } else {
                insert_result.unwrap();
            }
        }

        Ok(None)
    }

    fn cancel_order(
        &mut self,
        side: Side,
        order_id: &u128,
        expected_owner: &[u64; 4],
        expected_owner_slot: u8,
        client_order_id: Option<NonZeroU64>,

        event_q: &mut EventQueue,
    ) -> DexResult<()> {
        if let Some(leaf_node) = self.orders_mut(side).remove_by_key(order_id) {
            if leaf_node.owner() == expected_owner && leaf_node.owner_slot() == expected_owner_slot
            {
                if let Some(client_id) = client_order_id {
                    debug_assert_eq!(client_id.get(), leaf_node.client_order_id());
                }
                let native_qty_unlocked = match side {
                    Side::Bid => {
                        leaf_node.quantity()
                            * leaf_node.price().get()
                            * self.market_state.pc_lot_size
                    }
                    Side::Ask => leaf_node.quantity() * self.market_state.coin_lot_size,
                };
                event_q
                    .push_back(Event::new(EventView::Out {
                        side,
                        native_qty_unlocked,
                        native_qty_still_locked: 0,
                        order_id,
                        owner: expected_owner,
                        owner_slot: expected_owner_slot,
                        client_order_id: NonZeroU64::new(leaf_node.client_order_id()),
                    }))
                    .map_err(|_| DexErrorCode::EventQueueFull)?;
            } else {
                self.orders_mut(side).insert_leaf(&leaf_node).unwrap();
            }
        }
        Ok(())
    }
}
