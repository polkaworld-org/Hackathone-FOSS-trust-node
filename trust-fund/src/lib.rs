// Copyright 2017-2019 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

// Ensure we're `no_std` when compiling for Wasm.
#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Compact, CompactAs, Decode, Encode};
use rstd::{marker::PhantomData, prelude::*};
use sr_primitives::{
    traits::{Bounded, Convert, One, SignedExtension, Zero},
    transaction_validity::{
        InvalidTransaction, TransactionValidity, TransactionValidityError, ValidTransaction,
    },
    weights::{DispatchInfo, SimpleDispatchInfo},
    Perbill,
};
use support::{
    decl_event, decl_module, decl_storage, dispatch::Result, ensure, StorageMap, StorageValue,
};
use system::{ensure_root, ensure_signed};

/// Our module's configuration trait. All our types and consts go in here. If the
/// module is dependent on specific other modules, then their configuration traits
/// should be added to our implied traits list.
///
/// `system::Trait` should always be included in our implied traits.
pub trait Trait: balances::Trait + assets::Trait + timestamp::Trait + system::Trait {
    /// The overarching event type.
    type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;

    //    type ConvertBalance: Convert<BalanceOf<Self>, u128> + Convert<u128, BalanceOf<Self>>;
    type BalanceToU128: From<BalanceOf<Self>> + Into<u128>;
    type U128ToBalance: From<u128> + Into<BalanceOf<Self>>;
}

#[derive(Encode, Decode, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct BeneficiaryShare<AccountId> {
    address: AccountId,
    weight: u64,
}

#[derive(Encode, Decode, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub enum LivingSwitchCond<BlockNumber, Moment> {
    None,
    BlockHeight(BlockNumber),
    Timestamp(Moment),
    ClockInInterval(BlockNumber),
}

impl<BlockNumber, Moment> Default for LivingSwitchCond<BlockNumber, Moment> {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Encode, Decode, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
struct SchedulePayment<AssetId, AccountId, Balance> {
    asset_id: AssetId,
    receiver: AccountId,
    amount: Balance,
}

type BalanceOf<T> = <T as assets::Trait>::Balance;

decl_storage! {
    trait Store for Module<T: Trait> as TrustFund {
        Beneficiaries get(beneficiaries): map T::AccountId => Vec<BeneficiaryShare<T::AccountId>>;
        LivingSwitchConds get(living_switch_cond): map T::AccountId => LivingSwitchCond<T::BlockNumber, T::Moment>;
        LastClockIn get(last_clock_in): map T::AccountId => T::BlockNumber;
    }
}

decl_event!(
    /// Events are a simple means of reporting specific conditions and
    /// circumstances that have happened that users, Dapps and/or chain explorers would find
    /// interesting and otherwise difficult to detect.
    pub enum Event<T>
    where
        AccountId = <T as system::Trait>::AccountId,
        BlockNumber = <T as system::Trait>::BlockNumber,
        Moment = <T as timestamp::Trait>::Moment,
    {
        // Just a normal `enum`, here's a dummy event to ensure it compiles.
        BeneficiariesSet(AccountId, Vec<BeneficiaryShare<AccountId>>),
        LivingSwitchCondSet(AccountId, LivingSwitchCond<BlockNumber, Moment>),
        Withdraw(AccountId),
    }
);

decl_module! {
    // Simple declaration of the `Module` type. Lets the macro know what its working on.
    pub struct Module<T: Trait> for enum Call where origin: T::Origin {
        /// Deposit one of this module's events by using the default implementation.
        /// It is also possible to provide a custom implementation.
        /// For non-generic events, the generic parameter just needs to be dropped, so that it
        /// looks like: `fn deposit_event() = default;`.
        fn deposit_event() = default;

        fn deposit(origin, asset_id: T::AssetId, amount: BalanceOf<T>) -> Result {
            Ok(())
        }

        fn set_beneficiaries(origin, option: Vec<BeneficiaryShare<T::AccountId>>) -> Result {
            let grantor = ensure_signed(origin)?;
            <Beneficiaries<T>>::insert(&grantor, &option);
            Self::deposit_event(RawEvent::BeneficiariesSet(grantor, option));
            Ok(())
        }

        fn set_schedule_payment(origin, asset_id: T::AssetId, beneficiary: T::AccountId, amount: BalanceOf<T>) -> Result {
            Ok(())
        }

        fn stop_schedule_payment(origin, asset_id: T::AssetId, beneficiary: T::AccountId) -> Result {
            Ok(())
        }

        fn clock_in(origin) -> Result {
            let grantor = ensure_signed(origin)?;
            let block_number = <system::Module<T>>::block_number();
            <LastClockIn<T>>::insert(&grantor, &block_number);
            Ok(())
        }

        fn set_living_switch_condition(origin, condition: LivingSwitchCond<T::BlockNumber, T::Moment>) -> Result {
            let grantor = ensure_signed(origin)?;
            <LivingSwitchConds<T>>::insert(&grantor, &condition);
            Self::deposit_event(RawEvent::LivingSwitchCondSet(grantor, condition));
            Ok(())
        }

        fn withdraw(origin, grantor: T::AccountId, asset_id: T::AssetId) -> Result {
            let living_cond = <LivingSwitchConds<T>>::get(&grantor);

            let can_withdraw = Self::check_withdrawable(&grantor, &living_cond)?;
            ensure!(can_withdraw, "not withdrawable yet");
            let total_amount = <assets::Module<T>>::balance(asset_id.clone(), grantor.clone());
            ensure!(total_amount > Zero::zero(), "no balance");
            let beneficiaries = <Beneficiaries<T>>::get(&grantor);
            ensure!(beneficiaries.len() > Zero::zero(), "no beneficiaries");
            Self::calc_shares(&total_amount, &beneficiaries).iter().for_each(|share| match share {
                    (account, amount) => {
                        <assets::Module<T>>::make_transfer(grantor.clone(), asset_id.clone(), (*account).clone(), (*amount).clone());
                    }
                }
            );
            Self::deposit_event(RawEvent::Withdraw(grantor));
            Ok(())
        }

        // The signature could also look like: `fn on_initialize()`
        fn on_initialize(_n: T::BlockNumber) {
            // Anything that needs to be done at the start of the block.
            // We don't do anything here.
        }

        // The signature could also look like: `fn on_finalize()`
        fn on_finalize(_n: T::BlockNumber) {
            // Anything that needs to be done at the end of the block.
            // We just kill our dummy storage item.
//            <Dummy<T>>::kill();
        }

        // A runtime code run after every block and have access to extended set of APIs.
        //
        // For instance you can generate extrinsics for the upcoming produced block.
        fn offchain_worker(_n: T::BlockNumber) {
            // We don't do anything here.
            // but we could dispatch extrinsic (transaction/unsigned/inherent) using
            // runtime_io::submit_extrinsic
        }
    }
}

// The main implementation block for the module. Functions here fall into three broad
// categories:
// - Public interface. These are functions that are `pub` and generally fall into inspector
// functions that do not write to storage and operation functions that do.
// - Private functions. These are your usual private utilities unavailable to other modules.
impl<T: Trait> Module<T> {
    fn check_withdrawable(
        granter: &T::AccountId,
        cond: &LivingSwitchCond<T::BlockNumber, T::Moment>,
    ) -> rstd::result::Result<bool, &'static str> {
        match cond {
            LivingSwitchCond::None => Ok(false),
            LivingSwitchCond::BlockHeight(height) => {
                let block_number = <system::Module<T>>::block_number();
                Ok(block_number > *height)
            }
            LivingSwitchCond::Timestamp(end_date) => {
                let timestamp = <timestamp::Module<T>>::get();
                Ok(*end_date > timestamp)
            }
            LivingSwitchCond::ClockInInterval(interval) => {
                let last_clock_in = <LastClockIn<T>>::get(granter);
                let block_number = <system::Module<T>>::block_number();
                Ok((block_number - last_clock_in) > *interval)
            }
        }
    }

    fn calc_shares(
        amount: &BalanceOf<T>,
        beneficiaries: &Vec<BeneficiaryShare<T::AccountId>>,
    ) -> Vec<(T::AccountId, BalanceOf<T>)> {
        let to_balance = |b: u128| T::U128ToBalance::from(b).into();
        let to_u128 = |b: BalanceOf<T>| T::BalanceToU128::from(b).into();
        let total_weight = beneficiaries.iter().fold(0_u64, |acc, b| acc + b.weight);

        beneficiaries
            .iter()
            .map(|b| {
                let ration = Perbill::from_rational_approximation(b.weight, total_weight);
                (b.address.clone(), to_balance(ration * to_u128(*amount)))
            })
            .collect()
    }
}
