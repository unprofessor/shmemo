use log::{Level, Log, Metadata, Record};

pub struct MemoLogger;

impl Log for MemoLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            match record.level() {
                Level::Error => eprintln!(":: shmemo :: ERROR: {}", record.args()),
                Level::Warn => eprintln!(":: shmemo :: WARN: {}", record.args()),
                Level::Info => eprintln!(":: shmemo :: {}", record.args()),
                Level::Debug => eprintln!(":: shmemo :: DEBUG: {}", record.args()),
                Level::Trace => eprintln!(":: shmemo :: TRACE: {}", record.args()),
            }
        }
    }

    fn flush(&self) {}
}

pub fn init(level: log::LevelFilter) -> Result<(), log::SetLoggerError> {
    static LOGGER: MemoLogger = MemoLogger;
    log::set_logger(&LOGGER).map(|()| log::set_max_level(level))
}
