//! Build and run once: ./zbuild.py dev logging --profile debug
//! With configuration: ./zbuild.py dev logging --profile debug -- doc/server.json example
use std::fmt;

#[derive(Debug)]
struct Order {
    id: u64,
    symbol: &'static str,
    quantity: u32,
    price: f64,
}

impl fmt::Display for Order {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Order #{}: {} x {} @ {:.2}",
            self.id, self.quantity, self.symbol, self.price
        )
    }
}

// A borrowed view provides a log-specific format without copying the order.
struct OrderLog<'a>(&'a Order);

impl fmt::Display for OrderLog<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let order = self.0;
        write!(
            formatter,
            "order_id={} symbol={} quantity={} price={:.2}",
            order.id, order.symbol, order.quantity, order.price
        )
    }
}

fn log_scalar_types() {
    let signed8: i8 = -8;
    let signed16: i16 = -1600;
    let signed32: i32 = -32000;
    let signed64: i64 = -64000;
    let signed128: i128 = -123456789012345678901234567890;
    let signed_size: isize = -42;
    let unsigned8: u8 = 8;
    let unsigned16: u16 = 1600;
    let unsigned32: u32 = 32000;
    let unsigned64: u64 = 64000;
    let unsigned128: u128 = 123456789012345678901234567890;
    let unsigned_size: usize = 42;
    let float32: f32 = 1.25;
    let float64: f64 = 123.456789;
    let is_ready: bool = true;
    let character: char = '量';

    z::log::info!("[example:scalar] i8={signed8}");
    z::log::info!("[example:scalar] i16={signed16}");
    z::log::info!("[example:scalar] i32={signed32}");
    z::log::info!("[example:scalar] i64={signed64}");
    z::log::info!("[example:scalar] i128={signed128}");
    z::log::info!("[example:scalar] isize={signed_size}");
    z::log::info!("[example:scalar] u8={unsigned8} hex={unsigned8:#04x}");
    z::log::info!("[example:scalar] u16={unsigned16}");
    z::log::info!("[example:scalar] u32={unsigned32}");
    z::log::info!("[example:scalar] u64={unsigned64}");
    z::log::info!("[example:scalar] u128={unsigned128}");
    z::log::info!("[example:scalar] usize={unsigned_size}");
    z::log::info!("[example:scalar] f32={float32} precision={float32:.3}");
    z::log::info!("[example:scalar] f64={float64} precision={float64:.2}");
    z::log::info!("[example:scalar] bool={is_ready}");
    z::log::info!("[example:scalar] char={character} debug={character:?}");
}

fn log_composite_types() {
    let text: &str = "borrowed text";
    let owned_text: String = String::from("owned text");
    let array: [i32; 3] = [1, 2, 3];
    let slice: &[i32] = &array[1..];
    let tuple: (&str, u32, bool) = ("order", 3, true);
    let present: Option<u32> = Some(42);
    let absent: Option<u32> = None;
    let success: Result<u32, &str> = Ok(42);
    let failure: Result<u32, &str> = Err("demo rejection");

    z::log::info!("[example:composite] &str={text} debug={text:?}");
    z::log::info!("[example:composite] String={owned_text} debug={owned_text:?}");
    // These standard composite types use Debug rather than Display.
    z::log::info!("[example:composite] array={array:?}");
    z::log::info!("[example:composite] slice={slice:?}");
    z::log::info!("[example:composite] tuple={tuple:?}");
    z::log::info!("[example:composite] Option::Some={present:?}");
    z::log::info!("[example:composite] Option::None={absent:?}");
    z::log::info!("[example:composite] Result::Ok={success:?}");
    z::log::info!("[example:composite] Result::Err={failure:?}");
}

fn log_custom_formats() {
    let order = Order {
        id: 1001,
        symbol: "DEMO",
        quantity: 3,
        price: 123.45,
    };

    z::log::info!("[example:custom] Debug={order:?}");
    z::log::info!("[example:custom] Display={order}");
    z::log::info!("[example:custom] OrderLog={}", OrderLog(&order));
}

fn log_levels() {
    z::log::trace!("[example:trace] demo trace record");
    z::log::debug!("[example:debug] demo debug record");
    z::log::info!("[example:info] demo info record");
    z::log::warn!("[example:warn] demo warning; no actual failure");
    z::log::error!("[example:error] demo error; no actual failure");
    z::log::critical!("[example:critical] demo critical record; no actual failure");
    // Off is a filtering switch, not an emitting severity. Keep the supplied level.
    // important! emits at info, with independent routing to an enabled important sink.
    z::log::important!("[example:ready] service ready");
}

fn main() -> z::log::Result<()> {
    let config = match (std::env::args().nth(1), std::env::args().nth(2)) {
        (Some(path), Some(section)) => z::log::Config::from_zpp_file(path, &section)?,
        _ => z::log::Config::default(),
    };
    let guard = z::log::init(config)?;
    z::log::info!("[example:startup] service started");
    log_scalar_types();
    log_composite_types();
    log_custom_formats();
    log_levels();
    guard.shutdown()
}
