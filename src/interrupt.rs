//! The [`Interrupt`] flag that stops a running compiled function from another thread.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A flag that stops compiled code at its next loop iteration.
///
/// Compiled code runs on the caller's thread and cannot be preempted, so a function
/// that loops forever would hold that thread for good. A function compiled with
/// [`Jit::compile_interruptible`](crate::Jit::compile_interruptible) reads this flag at
/// the end of every loop iteration. Once the flag is set, the function returns at the
/// next check with the zero value of its return type: `0`, `0.0`, `false`, or nothing
/// for `unit`. That value is not the function's result, so after a call returns, check
/// [`is_interrupted`](Self::is_interrupted) before using it.
///
/// Code with no loop never checks the flag. It runs straight through, so its running
/// time is bounded by its size.
///
/// The flag is shared: clones refer to the same flag, so one clone can go to a
/// watchdog thread while the compiled functions hold another. It stays set until
/// [`clear`](Self::clear) is called, and while it is set every function compiled
/// against it returns at its first loop iteration.
///
/// # Examples
///
/// Stop a loop that would never end on its own:
///
/// ```
/// use jit_lang::{Interrupt, Jit};
/// use ir_lang::{Builder, Type};
///
/// // fn spin() -> int { loop {} }
/// let mut b = Builder::new("spin", &[], Type::Int);
/// let top = b.create_block(&[]);
/// b.jump(top, &[]);
/// b.switch_to(top);
/// b.jump(top, &[]);
///
/// let interrupt = Interrupt::new();
/// let jit = Jit::new().expect("the host is supported");
/// let spin = jit
///     .compile_interruptible(&b.finish(), &interrupt)
///     .expect("spin is well-formed");
///
/// let watchdog = interrupt.clone();
/// let timer = std::thread::spawn(move || {
///     std::thread::sleep(std::time::Duration::from_millis(10));
///     watchdog.interrupt();
/// });
///
/// // SAFETY: the signature is `fn() -> int`, and `spin` outlives the call.
/// let spin_fn: extern "C" fn() -> i64 = unsafe { spin.entry() };
/// assert_eq!(spin_fn(), 0);
/// assert!(interrupt.is_interrupted());
/// timer.join().unwrap();
/// ```
#[derive(Clone, Default)]
pub struct Interrupt {
    /// The flag byte compiled code loads. Reference-counted so every [`Compiled`]
    /// built against it keeps it alive for as long as its code can run.
    ///
    /// [`Compiled`]: crate::Compiled
    flag: Arc<AtomicBool>,
}

impl Interrupt {
    /// Creates a flag that is not set.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::Interrupt;
    ///
    /// assert!(!Interrupt::new().is_interrupted());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the flag. Every function compiled against it that is running, on any
    /// thread, returns at its next loop iteration, and later calls return at their
    /// first one until [`clear`](Self::clear).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::Interrupt;
    ///
    /// let interrupt = Interrupt::new();
    /// interrupt.interrupt();
    /// assert!(interrupt.is_interrupted());
    /// ```
    pub fn interrupt(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Clears the flag, so compiled code runs normally again.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::Interrupt;
    ///
    /// let interrupt = Interrupt::new();
    /// interrupt.interrupt();
    /// interrupt.clear();
    /// assert!(!interrupt.is_interrupted());
    /// ```
    pub fn clear(&self) {
        self.flag.store(false, Ordering::Release);
    }

    /// Whether the flag is set. After an interruptible call returns, `true` means the
    /// returned value may be the zero value from an interrupted run rather than the
    /// function's result.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit_lang::Interrupt;
    ///
    /// let interrupt = Interrupt::new();
    /// assert!(!interrupt.is_interrupted());
    /// ```
    #[must_use]
    pub fn is_interrupted(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// The address of the flag byte, baked into compiled code as a constant.
    /// `AtomicBool` has the same in-memory representation as a `bool`, one byte that
    /// is `0` or `1`.
    pub(crate) fn address(&self) -> usize {
        self.flag.as_ptr() as usize
    }
}

impl fmt::Debug for Interrupt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Interrupt")
            .field("interrupted", &self.is_interrupted())
            .finish()
    }
}
