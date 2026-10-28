//! The Resource governor's contracts (ADR 0040, spec #213): its settings,
//! every key under `governor.` (ADR 0083), the `engine.system_info` event
//! that carries its readings, its metrics, and the Error code its shedding
//! ladder ends a page with. One governor per engine process of either
//! Product, so everything here is Collie's.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Event, EventClass, Instrument, Metric,
    MetricLabel, ProtocolVersion, Setting, Stability, ValueKind, schema_of,
};
use crate::page::PageId;
use crate::settings::SettingLimit;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A kind of work the governor admits, each with its own floor and ceiling
/// (ADR 0040).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    /// An HTTP or discovered-API fetch, held mainly by CPU.
    Http,
    /// An open browser page, held mainly by memory.
    BrowserPage,
    /// A model call, held mainly by its provider's rate limits.
    ModelCall,
}

impl WorkKind {
    /// Every kind, in a fixed order.
    pub const ALL: [Self; 3] = [Self::Http, Self::BrowserPage, Self::ModelCall];

    /// Its name, as its settings, events and metric labels spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::BrowserPage => "browser_page",
            Self::ModelCall => "model_call",
        }
    }

    /// Its floor's setting.
    pub fn floor(self) -> &'static Setting {
        match self {
            Self::Http => &HTTP_FLOOR,
            Self::BrowserPage => &BROWSER_PAGE_FLOOR,
            Self::ModelCall => &MODEL_CALL_FLOOR,
        }
    }

    /// Its ceiling's setting.
    pub fn ceiling(self) -> &'static Setting {
        match self {
            Self::Http => &HTTP_CEILING,
            Self::BrowserPage => &BROWSER_PAGE_CEILING,
            Self::ModelCall => &MODEL_CALL_CEILING,
        }
    }
}

fn one() -> serde_json::Value {
    serde_json::json!(1)
}

fn unset() -> serde_json::Value {
    serde_json::Value::Null
}

fn two_hundred() -> serde_json::Value {
    serde_json::json!(200)
}

fn thirty_two() -> serde_json::Value {
    serde_json::json!(32)
}

fn sixteen() -> serde_json::Value {
    serde_json::json!(16)
}

fn eighty_five() -> serde_json::Value {
    serde_json::json!(85)
}

fn ninety_five() -> serde_json::Value {
    serde_json::json!(95)
}

fn one_second() -> serde_json::Value {
    serde_json::json!(1000)
}

fn sixty_seconds() -> serde_json::Value {
    serde_json::json!(60_000)
}

fn one_hundred() -> serde_json::Value {
    serde_json::json!(100)
}

fn eight() -> serde_json::Value {
    serde_json::json!(8)
}

fn one_gibibyte() -> serde_json::Value {
    serde_json::Value::from(1_u64 << 30)
}

fn five_minutes() -> serde_json::Value {
    serde_json::json!(300_000)
}

/// `governor.http.floor`.
pub const HTTP_FLOOR: Setting = Setting {
    name: "governor.http.floor",
    description: "The fewest HTTP and discovered-API fetches the engine's Resource governor keeps admitting at once, however loaded the machine: 1. Equal to `governor.http.ceiling`, it fixes the concurrency there.",
    kind: ValueKind::Count,
    default: one,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.http.ceiling`.
pub const HTTP_CEILING: Setting = Setting {
    name: "governor.http.ceiling",
    description: "The most HTTP and discovered-API fetches the engine's Resource governor admits at once, across its Runs: 200.",
    kind: ValueKind::Count,
    default: two_hundred,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.browser_page.floor`.
pub const BROWSER_PAGE_FLOOR: Setting = Setting {
    name: "governor.browser_page.floor",
    description: "The fewest browser pages the engine's Resource governor keeps admitting at once, however loaded the machine: 1. Equal to `governor.browser_page.ceiling`, it fixes the concurrency there.",
    kind: ValueKind::Count,
    default: one,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.browser_page.ceiling`.
pub const BROWSER_PAGE_CEILING: Setting = Setting {
    name: "governor.browser_page.ceiling",
    description: "The most browser pages the engine's Resource governor admits open at once, across its Runs: 32.",
    kind: ValueKind::Count,
    default: thirty_two,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.model_call.floor`.
pub const MODEL_CALL_FLOOR: Setting = Setting {
    name: "governor.model_call.floor",
    description: "The fewest model calls the engine's Resource governor keeps admitting at once, however loaded the machine: 1. Equal to `governor.model_call.ceiling`, it fixes the concurrency there.",
    kind: ValueKind::Count,
    default: one,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.model_call.ceiling`.
pub const MODEL_CALL_CEILING: Setting = Setting {
    name: "governor.model_call.ceiling",
    description: "The most model calls the engine's Resource governor admits at once, across its Runs: 16.",
    kind: ValueKind::Count,
    default: sixteen,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.memory_allowance` (ADR 0040; ADR 0083 names it).
pub const MEMORY_ALLOWANCE: Setting = Setting {
    name: "governor.memory_allowance",
    description: "The most memory, in bytes, the engine and its children may use, as its Memory allowance when it is tighter than the process's own cgroup limit, its Windows Job object's limit or half of physical memory; unset by default. Read again at every sample.",
    kind: ValueKind::Count,
    default: unset,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.pressure`.
pub const PRESSURE: Setting = Setting {
    name: "governor.pressure",
    description: "The share of the Memory allowance in use, in percent, from which the engine is under pressure and its Resource governor lowers its concurrency; also the CPU and Tokio workers' busy share from which they hold it: 85.",
    kind: ValueKind::Count,
    default: eighty_five,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.critical`.
pub const CRITICAL: Setting = Setting {
    name: "governor.critical",
    description: "The share of the Memory allowance in use, in percent, from which the engine's memory is critical: 95.",
    kind: ValueKind::Count,
    default: ninety_five,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.sample_interval`.
pub const SAMPLE_INTERVAL: Setting = Setting {
    name: "governor.sample_interval",
    description: "How often the engine's Resource governor reads memory, CPU and Tokio saturation, sizes its concurrency and sends `engine.system_info`: every second.",
    kind: ValueKind::Duration,
    default: one_second,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.last_resort_after`.
pub const LAST_RESORT_AFTER: Setting = Setting {
    name: "governor.last_resort_after",
    description: "How long the engine's memory may stay critical with its concurrency at the floor before the Resource governor reaches its last resort, when a Daemon's `/readyz` fails: 60 s.",
    kind: ValueKind::Duration,
    default: sixty_seconds,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.run_share`.
pub const RUN_SHARE: Setting = Setting {
    name: "governor.run_share",
    description: "The share, in percent, of each kind's concurrency one Run may hold at once, at least one: 100. An Operator policy cap bounds every Run's share.",
    kind: ValueKind::Count,
    default: one_hundred,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.browser.contexts_per_process` (ADR 0040, SA-09).
pub const BROWSER_CONTEXTS_PER_PROCESS: Setting = Setting {
    name: "governor.browser.contexts_per_process",
    description: "How many of a Run's Sessions share one browser process, each in a browser context of its own: 8. The Run's next Session past it gets a process of its own; Sessions of different Runs never share one.",
    kind: ValueKind::Count,
    default: eight,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.browser.recycle_pages` (ADR 0040).
pub const BROWSER_RECYCLE_PAGES: Setting = Setting {
    name: "governor.browser.recycle_pages",
    description: "How many pages a browser opens before it is recycled: 100. Recycled, it opens no new page, closes once its pages are done and is replaced, each of its Sessions keeping its cookies and storage.",
    kind: ValueKind::Count,
    default: one_hundred,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.browser.recycle_idle` (ADR 0040).
pub const BROWSER_RECYCLE_IDLE: Setting = Setting {
    name: "governor.browser.recycle_idle",
    description: "How long a browser may hold no open page before it closes, each of its Sessions keeping its cookies and storage for the browser that replaces it: 5 minutes.",
    kind: ValueKind::Duration,
    default: five_minutes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.browser.memory_cap` (ADR 0040).
pub const BROWSER_MEMORY_CAP: Setting = Setting {
    name: "governor.browser.memory_cap",
    description: "The memory, in bytes, of a browser and its processes past which it is recycled; unset by default, when the cap is twice what the browser used once it had opened its first page.",
    kind: ValueKind::Count,
    default: unset,
    clearable: true,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `governor.browser.page_memory_cap` (ADR 0040).
pub const BROWSER_PAGE_MEMORY_CAP: Setting = Setting {
    name: "governor.browser.page_memory_cap",
    description: "The memory, in bytes, of one page's JavaScript heap and the buffers it holds, past which the Resource governor ends the page while memory is critical: 1 GiB. The page ends at its safe point, once no call on it is in flight, and its calls raise `page_memory_exceeded`, which is retryable.",
    kind: ValueKind::Count,
    default: one_gibibyte,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// A page the Resource governor ended past its memory cap (ADR 0040).
pub const PAGE_MEMORY_EXCEEDED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "page_memory_exceeded",
    description: "Memory was critical and the page held more than `governor.browser.page_memory_cap`, so the Resource governor ended it once no call on it was in flight. The page is gone: open another and try again, which the governor admits once memory frees up.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `page_memory_exceeded` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageMemoryExceededDetails {
    /// The page the governor ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// What it held when measured, in bytes: its JavaScript heap and the
    /// buffers it held.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub memory: u64,
    /// `governor.browser.page_memory_cap`, with its value and layer (ADR
    /// 0079).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: SettingLimit,
}

/// How loaded the engine is, as its governor judges it.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GovernorLevel {
    /// Every reading has headroom: concurrency may rise.
    Normal,
    /// A reading passed `governor.pressure`: concurrency drops.
    Pressure,
    /// Memory passed `governor.critical`, or the host's is critical.
    Critical,
}

/// The reading that holds the engine above `normal`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PressureSource {
    /// The engine and its children's memory, against the Memory allowance.
    Memory,
    /// The host's own memory pressure, which counts as the engine's.
    HostMemory,
    /// The engine's CPU time, against its CPU allowance.
    Cpu,
    /// The Tokio workers' busy time.
    Tokio,
}

impl PressureSource {
    /// Every source, in a fixed order.
    pub const ALL: [Self; 4] = [Self::Memory, Self::HostMemory, Self::Cpu, Self::Tokio];

    /// Its name, as the metric's label spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::HostMemory => "host_memory",
            Self::Cpu => "cpu",
            Self::Tokio => "tokio",
        }
    }
}

/// What set the Memory allowance: the tightest of them (ADR 0040).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AllowanceSource {
    /// The process's own cgroup's memory limit, v1 or v2.
    Cgroup,
    /// The Windows Job object the process runs in.
    JobObject,
    /// `governor.memory_allowance`.
    Setting,
    /// Half of physical memory, when nothing tighter applies.
    Physical,
}

/// The engine's memory, as the governor read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryReading {
    /// The Memory allowance, in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub allowance: u64,
    /// What set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub allowance_source: AllowanceSource,
    /// What the engine and its children use, in bytes: PSS on Linux, so
    /// pages its browsers share count once.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub used: u64,
    /// What the browsers it counts use, in bytes, each process with its
    /// descendants (PSS on Linux), summed; part of `used`, 0 with none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browsers: u64,
    /// The host's own memory pressure: PSI and `MemAvailable` on Linux,
    /// the memory-pressure level on macOS, the low-memory notification on
    /// Windows.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: GovernorLevel,
}

/// The engine's CPU and Tokio workers, as the governor read them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CpuReading {
    /// How many threads the engine may run at once, its CPU allowance
    /// (`available_parallelism`, which honours a cgroup's quota).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub parallelism: u32,
    /// The engine's CPU time since the last sample, in percent of its
    /// allowance.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub busy: u32,
    /// The Tokio workers' busy time since the last sample, in percent.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tokio_busy: u32,
}

/// One kind's concurrency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KindConcurrency {
    /// Its floor.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub floor: u32,
    /// Its ceiling.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ceiling: u32,
    /// How many the governor admits at once now.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: u32,
    /// How many are admitted now.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub in_use: u32,
}

/// Every kind's concurrency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Concurrency {
    /// HTTP and discovered-API fetches.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub http: KindConcurrency,
    /// Open browser pages.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_page: KindConcurrency,
    /// Model calls.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub model_call: KindConcurrency,
}

impl Concurrency {
    /// `kind`'s concurrency.
    pub fn of(&self, kind: WorkKind) -> &KindConcurrency {
        match kind {
            WorkKind::Http => &self.http,
            WorkKind::BrowserPage => &self.browser_page,
            WorkKind::ModelCall => &self.model_call,
        }
    }
}

/// What the governor read and decided at one sample: the payload of
/// `engine.system_info`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SystemInfo {
    /// How loaded the engine is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub level: GovernorLevel,
    /// The reading that holds it above `normal`; none at `normal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pressure_source: Option<PressureSource>,
    /// Whether memory stayed critical at the floor past
    /// `governor.last_resort_after`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub last_resort: bool,
    /// Its memory.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub memory: MemoryReading,
    /// Its CPU and Tokio workers.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cpu: CpuReading,
    /// Each kind's concurrency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub concurrency: Concurrency,
}

/// `engine.system_info` (ADR 0040, spec #213's RT-14).
pub const SYSTEM_INFO: Event = Event {
    name: "engine.system_info",
    description: "The engine's Resource governor sampled: its Memory allowance and what set it, the memory the engine and its children use, the host's pressure, CPU and Tokio saturation, its level and what holds it there, and each kind's concurrency. Telemetry: a newer sample replaces an unsent one.",
    class: EventClass::Telemetry,
    payload: schema_of::<SystemInfo>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.governor.saturation`: how much of its concurrency target the
/// governor uses, on which the Helm chart autoscales (ADR 0055).
pub const SATURATION: Metric = Metric {
    name: "goodfetch.governor.saturation",
    description: "The share of the resource governor's concurrency target in use, from 0 to 1, the highest of its kinds', on which the Helm chart's autoscaler scales the Daemons (ADRs 0040 and 0055).",
    instrument: Instrument::Gauge,
    unit: "1",
    labels: &[],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.governor.target`: each kind's concurrency target.
pub const TARGET: Metric = Metric {
    name: "goodfetch.governor.target",
    description: "How many of each kind of work the resource governor admits at once now, between its floor and its ceiling (ADR 0040).",
    instrument: Instrument::Gauge,
    unit: "{admission}",
    labels: &[MetricLabel {
        name: "kind",
        values: &["http", "browser_page", "model_call"],
    }],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// `goodfetch.governor.pressure_source`: what holds the governor above
/// `normal`.
pub const PRESSURE_SOURCE: Metric = Metric {
    name: "goodfetch.governor.pressure_source",
    description: "1 for the reading that holds the resource governor above normal, 0 for the others: all 0 at normal (ADR 0040).",
    instrument: Instrument::Gauge,
    unit: "1",
    labels: &[MetricLabel {
        name: "source",
        values: &["memory", "host_memory", "cpu", "tokio"],
    }],
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// Every setting of the governor, each under `governor.`.
pub const SETTINGS: [Setting; 17] = [
    HTTP_FLOOR,
    HTTP_CEILING,
    BROWSER_PAGE_FLOOR,
    BROWSER_PAGE_CEILING,
    MODEL_CALL_FLOOR,
    MODEL_CALL_CEILING,
    MEMORY_ALLOWANCE,
    PRESSURE,
    CRITICAL,
    SAMPLE_INTERVAL,
    LAST_RESORT_AFTER,
    RUN_SHARE,
    BROWSER_CONTEXTS_PER_PROCESS,
    BROWSER_RECYCLE_PAGES,
    BROWSER_RECYCLE_IDLE,
    BROWSER_MEMORY_CAP,
    BROWSER_PAGE_MEMORY_CAP,
];

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[SYSTEM_INFO],
    handler_requests: &[],
    error_codes: &[PAGE_MEMORY_EXCEEDED],
    plugin_kinds: &[],
    metrics: &[SATURATION, TARGET, PRESSURE_SOURCE],
    settings: &SETTINGS,
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
