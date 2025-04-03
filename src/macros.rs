/// Set a specific bit in `x`
macro_rules! set_bits_u8 {
    ($x:expr, $bit:expr) => {{
        $x | $bit
    }};
}

/// Clear (unset) a specific bit in `x`
macro_rules! clear_bits_u8 {
    ($x:expr, $bit:expr) => {{
        $x & !$bit
    }};
}

/// Toggle (flip) a specific bit in `x`
macro_rules! toggle_bits_u8 {
    ($x:expr, $bit:expr) => {{
        $x ^ $bit
    }};
}

/// Check if a specific bit is set in `x`
macro_rules! check_bits_u8 {
    ($x:expr, $bit:expr) => {
        ($x & $bit) == $bit
    };
}

/// Set a specific bit in a `Cell<u8>`
macro_rules! set_bits_cell {
    ($x:expr, $bit:expr) => {{
        $x.set(set_bits_u8!($x.get(), $bit));
        $x.get()
    }};
}

/// Clear (unset) a specific bit in a `Cell<u8>`
macro_rules! clear_bits_cell {
    ($x:expr, $bit:expr) => {{
        $x.set(clear_bits_u8!($x.get(), $bit));
        $x.get()
    }};
}

/// Toggle (flip) a specific bit in a `Cell<u8>`
macro_rules! toggle_bits_cell {
    ($x:expr, $bit:expr) => {{
        $x.set(toggle_bits_u8!($x.get(), $bit));
        $x.get()
    }};
}

/// Check if a specific bit is set in a `Cell<u8>`
macro_rules! check_bits_cell {
    ($x:expr, $bit:expr) => {
        check_bits_u8!($x.get(), $bit)
    };
}
