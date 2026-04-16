use log::{Level, Log, Metadata, Record};

pub struct MemoLogger;

impl Log for MemoLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            match record.level() {
                Level::Error => eprintln!(":: memo :: ERROR: {}", record.args()),
                Level::Warn => eprintln!(":: memo :: WARN: {}", record.args()),
                Level::Info => eprintln!(":: memo :: {}", record.args()),
                Level::Debug => eprintln!(":: memo :: DEBUG: {}", record.args()),
                Level::Trace => eprintln!(":: memo :: TRACE: {}", record.args()),
            }
        }
    }

    fn flush(&self) {}
}

pub fn init(level: log::LevelFilter) -> Result<(), log::SetLoggerError> {
    static LOGGER: MemoLogger = MemoLogger;
    log::set_logger(&LOGGER).map(|()| log::set_max_level(level))
}
