// A Runtime can delegate any transaction, run at specific block height
// 1) Transaction is executed on delegator's behalf
//  if it's too hard to support delegating any method, we can implement a version supporting asset transfer only
// 2) Not vulnerable to attack
//
// storage design
// 1. ScheduleTask get(schedule_task): map T::Hash => Option<Task<T>>;
// to store delegated task on chain
// struct Task<T: Trait> {
//    method: T::Hash,
//    sender: T::AccountId,
//    nonce: T::Index,
//    block_number: T::BlockNumber, // after which block height to run
//    next: T::Hash, // point to next task
// }
//
// 2. TaskLinkedByBlock get(block_linked_task): map T::BlockNumber => Option<LinkedTasks<T>>;
// struct LinkedTasks<T: Trait> {
//    head: T::Hash,
//    tail: T::Hash,
// }
//
//
// 3. NextNonce get(next_nonce): map T::AccountId => T::Index;
//
// 4. a config how many tasks at most to execute in one block
//
// Operation
// Insert a new task
// 1) Verify account nonce
// 2) Bump nonce
// 3) query TasksByBlock by block number,
//     if None, insert Some(Vec<TaskOf<T>>)
//     if Some, get Vec<TaskOf<T>> and
//
// Run tasks (triggered after each block)
// 1) query TaskLinkedByBlock by current block number,
//     if Some, move head after the tail of TaskLinkedByBlock key=0 then clean this key
// 2) query TaskLinkedByBlock key = 0 (pending tasks), pick first n tasks (according to config), and update head to the next pending task.
// 3) dispatch picked tasks to executor
// 4) write events to track

#![cfg_attr(not(feature = "std"), no_std)]

#[macro_use]
extern crate support;

use rstd::prelude::*;

use codec::{Decode, Encode, Codec};
use sr_primitives::traits::{Bounded, OffchainWorker, One, Zero, Dispatchable};

use support::{decl_event, decl_module, decl_storage, dispatch::{Result, Callable, Dispatchable as SupportDispatchable}, StorageMap, StorageValue};
use system::ensure_signed;

#[derive(Encode, Decode, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct Task<Call, AccountId, Index, BlockNumber> {
    method: Call,
    sender: AccountId,
    nonce: Index,
    block_number: BlockNumber,
}

pub type TaskOf<T> = Task<<T as system::Trait>::Call, <T as system::Trait>::AccountId, <T as system::Trait>::Index, <T as system::Trait>::BlockNumber>;

pub trait Trait: system::Trait {
    type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
}

// This module's storage items.
decl_storage! {
    trait Store for Module<T: Trait> as TemplateModule {
		pub TasksByBlock get(tasks_by_block): map T::BlockNumber => Option<Vec<TaskOf<T>>>;
        pub NextNonce get(next_nonce): map T::AccountId => T::Index;
    }
}

// The module's dispatchable functions.
decl_module! {
    /// The module declaration.
    pub struct Module<T: Trait> for enum Call where origin: T::Origin {
        // Initializing events
        // this is needed only if you are using events in your module
        fn deposit_event() = default;

		/// Run tasks.
        fn offchain_worker(block_number: T::BlockNumber) {
			let tasks = match <TasksByBlock<T>>::take(&block_number) {
				Some(t) => t,
				None => return,
			};

			tasks.into_iter().for_each(|t| {
				let origin = T::Origin::from(system::RawOrigin::Signed(t.sender.clone()));
				if let Ok(_) = t.method.clone().dispatch(origin) {
					Self::deposit_event(RawEvent::TaskExecutedOk(block_number, t.sender, t.nonce, t.method));
				} else {
					Self::deposit_event(RawEvent::TaskExecutedErr(block_number, t.sender, t.nonce, t.method));
				}
			})
        }
    }
}

decl_event!(
    pub enum Event<T>
    where
        AccountId = <T as system::Trait>::AccountId,
		BlockNumber = <T as system::Trait>::BlockNumber,
		Nonce = <T as system::Trait>::Index,
		Method = <T as system::Trait>::Call,
    {
		/// (block_number, who, nonce, method)
		TaskExecutedOk(BlockNumber, AccountId, Nonce, Method),
		TaskExecutedErr(BlockNumber, AccountId, Nonce, Method),
    }
);

impl<T: Trait> Module<T> {
	/// Schedule a task.
	pub fn schedule_task(task: TaskOf<T>) -> Result {
		let expected_nonce = <NextNonce<T>>::get(&task.sender);
		if task.nonce != expected_nonce {
			return Err("invalid nonce");
		}

		Self::inc_account_nonce(&task.sender);

		let block_number = task.block_number;
		let tasks = if let Some(mut tasks) = <TasksByBlock<T>>::take(&task.block_number) {
			tasks.push(task);
			tasks
		} else {
			vec!(task)
		};

		<TasksByBlock<T>>::insert(block_number, tasks);

		Ok(())
	}

    /// Increment a particular account's nonce by 1.
    pub fn inc_account_nonce(who: &T::AccountId) {
        <NextNonce<T>>::insert(who, Self::next_nonce(who) + T::Index::one());
    }
}
