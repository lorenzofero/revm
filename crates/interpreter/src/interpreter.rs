pub mod analysis;
mod contract;
pub(crate) mod shared_memory;
mod stack;

pub use analysis::BytecodeLocked;
pub use contract::Contract;
pub use shared_memory::SharedMemory;
pub use stack::{Stack, STACK_LIMIT};

use crate::primitives::{Bytes, Spec};
use crate::{
    alloc::boxed::Box,
    instructions::{eval, InstructionResult},
    Gas, Host,
};
use alloc::rc::Rc;
use core::cell::RefCell;
use core::ops::Range;

pub const CALL_STACK_LIMIT: u64 = 1024;

/// EIP-170: Contract code size limit
///
/// By default this limit is 0x6000 (~25kb)
pub const MAX_CODE_SIZE: usize = 0x6000;

/// EIP-3860: Limit and meter initcode
pub const MAX_INITCODE_SIZE: usize = 2 * MAX_CODE_SIZE;

pub struct Interpreter {
    /// Instruction pointer.
    pub instruction_pointer: *const u8,
    /// Return is main control flag, it tell us if we should continue interpreter or break from it
    pub instruction_result: InstructionResult,
    /// left gas. Memory gas can be found in Memory field.
    pub gas: Gas,
    /// Shared memory.
    pub shared_memory: Rc<RefCell<SharedMemory>>,
    /// Stack.
    pub stack: Stack,
    /// After call returns, its return data is saved here.
    pub return_data_buffer: Bytes,
    /// Return value.
    pub return_range: Range<usize>,
    /// Is interpreter call static.
    pub is_static: bool,
    /// Contract information and invoking data
    pub contract: Box<Contract>,
}

impl Interpreter {
    /// Current opcode
    pub fn current_opcode(&self) -> u8 {
        unsafe { *self.instruction_pointer }
    }

    /// Create new interpreter
    pub fn new(
        contract: Box<Contract>,
        gas_limit: u64,
        is_static: bool,
        shared_memory: &Rc<RefCell<SharedMemory>>,
    ) -> Self {
        Self {
            instruction_pointer: contract.bytecode.as_ptr(),
            return_range: Range::default(),
            stack: Stack::new(),
            shared_memory: Rc::clone(shared_memory),
            return_data_buffer: Bytes::new(),
            contract,
            instruction_result: InstructionResult::Continue,
            is_static,
            gas: Gas::new(gas_limit),
        }
    }

    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    pub fn gas(&self) -> &Gas {
        &self.gas
    }

    /// Reference of interpreter stack.
    pub fn stack(&self) -> &Stack {
        &self.stack
    }

    /// Return a reference of the program counter.
    pub fn program_counter(&self) -> usize {
        // Safety: this is just subtraction of pointers, it is safe to do.
        unsafe {
            self.instruction_pointer
                .offset_from(self.contract.bytecode.as_ptr()) as usize
        }
    }

    /// Execute next instruction
    #[inline(always)]
    pub fn step<H: Host, SPEC: Spec>(&mut self, host: &mut H) {
        // step.
        let opcode = unsafe { *self.instruction_pointer };
        // Safety: In analysis we are doing padding of bytecode so that we are sure that last
        // byte instruction is STOP so we are safe to just increment program_counter bcs on last instruction
        // it will do noop and just stop execution of this contract
        self.instruction_pointer = unsafe { self.instruction_pointer.offset(1) };
        eval::<H, SPEC>(opcode, self, host);
    }

    /// loop steps until we are finished with execution
    pub fn run<H: Host, SPEC: Spec>(&mut self, host: &mut H) -> InstructionResult {
        while self.instruction_result == InstructionResult::Continue {
            self.step::<H, SPEC>(host)
        }
        self.instruction_result
    }

    /// loop steps until we are finished with execution
    pub fn run_inspect<H: Host, SPEC: Spec>(&mut self, host: &mut H) -> InstructionResult {
        while self.instruction_result == InstructionResult::Continue {
            // step
            let ret = host.step(self);
            if ret != InstructionResult::Continue {
                return ret;
            }
            self.step::<H, SPEC>(host);

            // step ends
            let ret = host.step_end(self, self.instruction_result);
            if ret != InstructionResult::Continue {
                return ret;
            }
        }
        self.instruction_result
    }

    /// Copy and get the return value of the interpreter, if any.
    pub fn return_value(&self) -> Bytes {
        // if start is usize max it means that our return len is zero and we need to return empty
        let bytes = if self.return_range.start == usize::MAX {
            Bytes::new()
        } else {
            Bytes::copy_from_slice(self.shared_memory.borrow().get_slice(
                self.return_range.start,
                self.return_range.end - self.return_range.start,
            ))
        };
        self.shared_memory.borrow_mut().free_context_memory();
        bytes
    }
}
