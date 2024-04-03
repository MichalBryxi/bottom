//! How to handle config files and arguments.

// TODO: Break this apart or do something a bit smarter.

pub mod args;
pub mod config;

use std::{convert::TryInto, str::FromStr, time::Instant};

use anyhow::{Context, Result};
use hashbrown::{HashMap, HashSet};
use indexmap::IndexSet;
use regex::Regex;
#[cfg(feature = "battery")]
use starship_battery::Manager;

use self::{
    args::StringOrNum,
    config::{layout::Row, ConfigV2, IgnoreList},
};
use crate::{
    app::{filter::Filter, layout_manager::*, *},
    canvas::{styling::CanvasStyling, ColourScheme},
    constants::*,
    data_collection::temperature::TemperatureType,
    utils::{
        data_units::DataUnit,
        error::{self, BottomError},
    },
    widgets::*,
};

pub fn init_app(
    config: ConfigV2, widget_layout: &BottomLayout, default_widget_id: u64,
    default_widget_type_option: &Option<BottomWidgetType>, styling: &CanvasStyling,
) -> Result<App> {
    use BottomWidgetType::*;

    let retention_ms = get_retention(&config).context("Update `retention` in your config file.")?;
    let autohide_time = config.general.args.autohide_time.unwrap_or(false);
    let default_time_value = get_default_time_value(&config, retention_ms)?;

    let use_basic_mode = config.general.args.basic.unwrap_or(false);
    let expanded_upon_startup = config.general.args.expanded.unwrap_or(false);

    // For processes
    let is_grouped = config.process.args.group_processes.unwrap_or(false);
    let is_case_sensitive = config.process.args.case_sensitive.unwrap_or(false);
    let is_match_whole_word = config.process.args.whole_word.unwrap_or(false);
    let is_use_regex = config.process.args.regex.unwrap_or(false);
    let show_memory_as_values = config.process.args.mem_as_value.unwrap_or(false);
    let is_default_tree = config.process.args.tree.unwrap_or(false);
    let is_default_command = config.process.args.process_command.unwrap_or(false);
    let is_advanced_kill = !(config.process.args.disable_advanced_kill.unwrap_or(false));

    let mut widget_map = HashMap::new();
    let mut cpu_state_map: HashMap<u64, CpuWidgetState> = HashMap::new();
    let mut mem_state_map: HashMap<u64, MemWidgetState> = HashMap::new();
    let mut net_state_map: HashMap<u64, NetWidgetState> = HashMap::new();
    let mut proc_state_map: HashMap<u64, ProcWidgetState> = HashMap::new();
    let mut temp_state_map: HashMap<u64, TempWidgetState> = HashMap::new();
    let mut disk_state_map: HashMap<u64, DiskTableWidget> = HashMap::new();
    let mut battery_state_map: HashMap<u64, BatteryWidgetState> = HashMap::new();

    let autohide_timer = if autohide_time {
        Some(Instant::now())
    } else {
        None
    };

    let mut initial_widget_id: u64 = default_widget_id;
    let mut initial_widget_type = Proc;
    let is_custom_layout = config.row.is_some();
    let mut used_widget_set = HashSet::new();

    let network_unit_type = if config.network.args.network_use_bytes.unwrap_or(false) {
        DataUnit::Byte
    } else {
        DataUnit::Bit
    };
    let network_scale_type = if config.network.args.network_use_log.unwrap_or(false) {
        AxisScaling::Log
    } else {
        AxisScaling::Linear
    };
    let network_use_binary_prefix = config
        .network
        .args
        .network_use_binary_prefix
        .unwrap_or(false);

    let proc_columns: Option<IndexSet<ProcWidgetColumn>> = {
        let columns = config.process.columns.as_ref();

        match columns {
            Some(columns) => {
                if columns.is_empty() {
                    None
                } else {
                    Some(IndexSet::from_iter(columns.clone()))
                }
            }
            None => None,
        }
    };

    // TODO: Can probably just reuse the options struct.
    let app_config_fields = AppConfigFields {
        update_rate: get_update_rate(&config)?,
        temperature_type: get_temperature(&config)?,
        show_average_cpu: !config.cpu.args.hide_avg_cpu.unwrap_or(false),
        use_dot: config.general.args.dot_marker.unwrap_or(false),
        left_legend: config.cpu.args.left_legend.unwrap_or(false),
        use_current_cpu_total: config.process.args.current_usage.unwrap_or(false),
        unnormalized_cpu: config.process.args.unnormalized_cpu.unwrap_or(false),
        use_basic_mode,
        default_time_value,
        time_interval: get_time_interval(&config, retention_ms)?,
        hide_time: config.general.args.hide_time.unwrap_or(false),
        autohide_time,
        use_old_network_legend: config.network.args.use_old_network_legend.unwrap_or(false),
        table_gap: u16::from(!(config.general.args.hide_table_gap.unwrap_or(false))),
        disable_click: config.general.args.disable_click.unwrap_or(false),
        enable_cache_memory: get_enable_cache_memory(&config),
        show_table_scroll_position: config
            .general
            .args
            .show_table_scroll_position
            .unwrap_or(false),
        is_advanced_kill,
        network_scale_type,
        network_unit_type,
        network_use_binary_prefix,
        retention_ms,
    };

    let table_config = ProcTableConfig {
        is_case_sensitive,
        is_match_whole_word,
        is_use_regex,
        show_memory_as_values,
        is_command: is_default_command,
    };

    for row in &widget_layout.rows {
        for col in &row.children {
            for col_row in &col.children {
                for widget in &col_row.children {
                    widget_map.insert(widget.widget_id, widget.clone());
                    if let Some(default_widget_type) = &default_widget_type_option {
                        if !is_custom_layout || use_basic_mode {
                            match widget.widget_type {
                                BasicCpu => {
                                    if let Cpu = *default_widget_type {
                                        initial_widget_id = widget.widget_id;
                                        initial_widget_type = Cpu;
                                    }
                                }
                                BasicMem => {
                                    if let Mem = *default_widget_type {
                                        initial_widget_id = widget.widget_id;
                                        initial_widget_type = Cpu;
                                    }
                                }
                                BasicNet => {
                                    if let Net = *default_widget_type {
                                        initial_widget_id = widget.widget_id;
                                        initial_widget_type = Cpu;
                                    }
                                }
                                _ => {
                                    if *default_widget_type == widget.widget_type {
                                        initial_widget_id = widget.widget_id;
                                        initial_widget_type = widget.widget_type.clone();
                                    }
                                }
                            }
                        }
                    }

                    used_widget_set.insert(widget.widget_type.clone());

                    match widget.widget_type {
                        Cpu => {
                            cpu_state_map.insert(
                                widget.widget_id,
                                CpuWidgetState::new(
                                    &app_config_fields,
                                    config.cpu.args.default_cpu_entry,
                                    default_time_value,
                                    autohide_timer,
                                    styling,
                                ),
                            );
                        }
                        Mem => {
                            mem_state_map.insert(
                                widget.widget_id,
                                MemWidgetState::init(default_time_value, autohide_timer),
                            );
                        }
                        Net => {
                            net_state_map.insert(
                                widget.widget_id,
                                NetWidgetState::init(default_time_value, autohide_timer),
                            );
                        }
                        Proc => {
                            let mode = if is_grouped {
                                ProcWidgetMode::Grouped
                            } else if is_default_tree {
                                ProcWidgetMode::Tree {
                                    collapsed_pids: Default::default(),
                                }
                            } else {
                                ProcWidgetMode::Normal
                            };

                            proc_state_map.insert(
                                widget.widget_id,
                                ProcWidgetState::new(
                                    &app_config_fields,
                                    mode,
                                    table_config,
                                    styling,
                                    &proc_columns,
                                ),
                            );
                        }
                        Disk => {
                            disk_state_map.insert(
                                widget.widget_id,
                                DiskTableWidget::new(&app_config_fields, styling),
                            );
                        }
                        Temp => {
                            temp_state_map.insert(
                                widget.widget_id,
                                TempWidgetState::new(&app_config_fields, styling),
                            );
                        }
                        Battery => {
                            battery_state_map
                                .insert(widget.widget_id, BatteryWidgetState::default());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let basic_table_widget_state = if use_basic_mode {
        Some(match initial_widget_type {
            Proc | Disk | Temp => BasicTableWidgetState {
                currently_displayed_widget_type: initial_widget_type,
                currently_displayed_widget_id: initial_widget_id,
                widget_id: 100,
                left_tlc: None,
                left_brc: None,
                right_tlc: None,
                right_brc: None,
            },
            _ => BasicTableWidgetState {
                currently_displayed_widget_type: Proc,
                currently_displayed_widget_id: DEFAULT_WIDGET_ID,
                widget_id: 100,
                left_tlc: None,
                left_brc: None,
                right_tlc: None,
                right_brc: None,
            },
        })
    } else {
        None
    };

    let use_mem = used_widget_set.get(&Mem).is_some() || used_widget_set.get(&BasicMem).is_some();
    let used_widgets = UsedWidgets {
        use_cpu: used_widget_set.get(&Cpu).is_some() || used_widget_set.get(&BasicCpu).is_some(),
        use_mem,
        use_cache: use_mem && get_enable_cache_memory(&config),
        use_gpu: {
            #[cfg(feature = "gpu")]
            {
                config.gpu.enabled()
            }
            #[cfg(not(feature = "gpu"))]
            {
                false
            }
        },
        use_net: used_widget_set.get(&Net).is_some() || used_widget_set.get(&BasicNet).is_some(),
        use_proc: used_widget_set.get(&Proc).is_some(),
        use_disk: used_widget_set.get(&Disk).is_some(),
        use_temp: used_widget_set.get(&Temp).is_some(),
        use_battery: used_widget_set.get(&Battery).is_some(),
    };

    let disk_filter =
        get_ignore_list(&config.disk_filter).context("Update 'disk_filter' in your config file")?;
    let mount_filter = get_ignore_list(&config.mount_filter)
        .context("Update 'mount_filter' in your config file")?;
    let temp_filter =
        get_ignore_list(&config.temp_filter).context("Update 'temp_filter' in your config file")?;
    let net_filter =
        get_ignore_list(&config.net_filter).context("Update 'net_filter' in your config file")?;

    let states = AppWidgetStates {
        cpu_state: CpuState::init(cpu_state_map),
        mem_state: MemState::init(mem_state_map),
        net_state: NetState::init(net_state_map),
        proc_state: ProcState::init(proc_state_map),
        temp_state: TempState::init(temp_state_map),
        disk_state: DiskState::init(disk_state_map),
        battery_state: BatteryState::init(battery_state_map),
        basic_table_widget_state,
    };

    let current_widget = widget_map.get(&initial_widget_id).unwrap().clone();
    let filters = DataFilters {
        disk_filter,
        mount_filter,
        temp_filter,
        net_filter,
    };
    let is_expanded = expanded_upon_startup && !use_basic_mode;

    Ok(App::new(
        app_config_fields,
        states,
        widget_map,
        current_widget,
        used_widgets,
        filters,
        is_expanded,
    ))
}

pub fn get_widget_layout(
    config: &ConfigV2,
) -> error::Result<(BottomLayout, u64, Option<BottomWidgetType>)> {
    let left_legend = config.cpu.args.left_legend.unwrap_or(false);

    let (default_widget_type, mut default_widget_count) = get_default_widget_and_count(config)?;
    let mut default_widget_id = 1;

    let bottom_layout = if config.general.args.basic.unwrap_or(false) {
        default_widget_id = DEFAULT_WIDGET_ID;

        BottomLayout::init_basic_default(get_use_battery(config))
    } else {
        let ref_row: Vec<Row>; // Required to handle reference
        let rows = match &config.row {
            Some(r) => r,
            None => {
                // This cannot (like it really shouldn't) fail!
                ref_row = toml_edit::de::from_str::<ConfigV2>(if get_use_battery(config) {
                    DEFAULT_BATTERY_LAYOUT
                } else {
                    DEFAULT_LAYOUT
                })?
                .row
                .unwrap();
                &ref_row
            }
        };

        let mut iter_id = 0; // A lazy way of forcing unique IDs *shrugs*
        let mut total_height_ratio = 0;

        let mut ret_bottom_layout = BottomLayout {
            rows: rows
                .iter()
                .map(|row| {
                    row.convert_row_to_bottom_row(
                        &mut iter_id,
                        &mut total_height_ratio,
                        &mut default_widget_id,
                        &default_widget_type,
                        &mut default_widget_count,
                        left_legend,
                    )
                })
                .collect::<error::Result<Vec<_>>>()?,
            total_row_height_ratio: total_height_ratio,
        };

        // Confirm that we have at least ONE widget left - if not, error out!
        if iter_id > 0 {
            ret_bottom_layout.get_movement_mappings();
            ret_bottom_layout
        } else {
            return Err(BottomError::ConfigError(
                "please have at least one widget under the '[[row]]' section.".to_string(),
            ));
        }
    };

    Ok((bottom_layout, default_widget_id, default_widget_type))
}

#[inline]
fn try_parse_ms(s: &str) -> error::Result<u64> {
    if let Ok(val) = humantime::parse_duration(s) {
        Ok(val.as_millis().try_into()?)
    } else if let Ok(val) = s.parse::<u64>() {
        Ok(val)
    } else {
        Err(BottomError::ConfigError(
            "could not parse as a valid 64-bit unsigned integer or a human time".to_string(),
        ))
    }
}

#[inline]
fn get_duration(
    value: &Option<StringOrNum>, min: u64, max: Option<u64>, default: u64,
    what_to_fix: &'static str,
) -> error::Result<u64> {
    if let Some(value) = value {
        let value = match value {
            StringOrNum::String(s) => try_parse_ms(s)?,
            StringOrNum::Num(n) => *n,
        };

        if value < min {
            return Err(BottomError::ConfigError(format!(
                "set your {what_to_fix} to be at least {min} ms."
            )));
        }

        if let Some(max) = max {
            if value > max {
                return Err(BottomError::ConfigError(format!(
                    "set your {what_to_fix} to be less than {max} ms."
                )));
            }
        }

        Ok(value)
    } else {
        Ok(default)
    }
}

#[inline]
fn get_update_rate(config: &ConfigV2) -> error::Result<u64> {
    get_duration(
        &config.general.args.rate,
        250,
        None,
        DEFAULT_REFRESH_RATE_IN_MILLISECONDS,
        "update rate",
    )
}

fn get_temperature(config: &ConfigV2) -> error::Result<TemperatureType> {
    if config.temperature.args.celsius {
        Ok(TemperatureType::Celsius)
    } else if config.temperature.args.fahrenheit {
        Ok(TemperatureType::Fahrenheit)
    } else if config.temperature.args.kelvin {
        Ok(TemperatureType::Kelvin)
    } else if let Some(temp_type) = &config.temperature.temperature_type {
        match temp_type.as_str() {
            "fahrenheit" | "f" => Ok(TemperatureType::Fahrenheit),
            "kelvin" | "k" => Ok(TemperatureType::Kelvin),
            "celsius" | "c" => Ok(TemperatureType::Celsius),
            _ => Err(BottomError::ConfigError(format!(
                "\"{temp_type}\" is an invalid temperature type, use \"<kelvin|k|celsius|c|fahrenheit|f>\"."
            ))),
        }
    } else {
        Ok(TemperatureType::Celsius)
    }
}

fn get_default_time_value(config: &ConfigV2, retention_ms: u64) -> error::Result<u64> {
    get_duration(
        &config.general.args.default_time_value,
        30000,
        Some(retention_ms),
        DEFAULT_TIME_MILLISECONDS,
        "default value",
    )
}

fn get_time_interval(config: &ConfigV2, retention_ms: u64) -> error::Result<u64> {
    get_duration(
        &config.general.args.time_delta,
        1000,
        Some(retention_ms),
        TIME_CHANGE_MILLISECONDS,
        "time delta",
    )
}

fn get_default_widget_and_count(
    config: &ConfigV2,
) -> error::Result<(Option<BottomWidgetType>, u64)> {
    let widget_type = config
        .general
        .args
        .default_widget_type
        .as_ref()
        .map(|widget| widget.parse::<BottomWidgetType>())
        .transpose()?;

    let widget_count: Option<u64> = config
        .general
        .args
        .default_widget_count
        .map(|count| count.into());

    match (widget_type, widget_count) {
        (Some(widget_type), Some(widget_count)) => {
            Ok((Some(widget_type), widget_count))
        }
        (Some(widget_type), None) => Ok((Some(widget_type), 1)),
        (None, Some(_widget_count)) =>  Err(BottomError::ConfigError(
            "cannot set `default_widget_count` by itself, it must be used with `default_widget_type`.".to_string(),
        )),
        (None, None) => Ok((None, 1))
    }
}

#[allow(unused_variables)]
fn get_use_battery(config: &ConfigV2) -> bool {
    #[cfg(feature = "battery")]
    {
        if let Ok(battery_manager) = Manager::new() {
            if let Ok(batteries) = battery_manager.batteries() {
                if batteries.count() == 0 {
                    return false;
                }
            }
        }

        if let Some(use_battery) = config.battery.args.battery {
            return use_battery;
        }
    }

    false
}

#[allow(unused_variables)]
fn get_enable_cache_memory(config: &ConfigV2) -> bool {
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(val) = config.memory.args.enable_cache_memory {
            return val;
        }
    }

    false
}

fn get_ignore_list(ignore_list: &Option<IgnoreList>) -> error::Result<Option<Filter>> {
    if let Some(ignore_list) = ignore_list {
        let list: Result<Vec<_>, _> = ignore_list
            .list
            .iter()
            .map(|name| {
                let escaped_string: String;
                let res = format!(
                    "{}{}{}{}",
                    if ignore_list.whole_word { "^" } else { "" },
                    if ignore_list.case_sensitive {
                        ""
                    } else {
                        "(?i)"
                    },
                    if ignore_list.regex {
                        name
                    } else {
                        escaped_string = regex::escape(name);
                        &escaped_string
                    },
                    if ignore_list.whole_word { "$" } else { "" },
                );

                Regex::new(&res)
            })
            .collect();

        Ok(Some(Filter {
            list: list?,
            is_list_ignored: ignore_list.is_list_ignored,
        }))
    } else {
        Ok(None)
    }
}

/// Get the colour scheme from the config if valid.
pub fn get_color_scheme(config: &ConfigV2) -> error::Result<ColourScheme> {
    if let Some(color) = &config.style.args.color {
        match ColourScheme::from_str(color) {
            Ok(scheme) => match scheme {
                ColourScheme::Custom => {
                    if let Some(colors) = &config.colors {
                        if !colors.is_empty() {
                            return Ok(ColourScheme::Custom);
                        }
                    }

                    Err(error::BottomError::ConfigError(
                        "empty custom color scheme defined".into(),
                    ))
                }
                _ => Ok(scheme),
            },
            Err(err) => Err(err),
        }
    } else if let Some(colors) = &config.colors {
        if !colors.is_empty() {
            Ok(ColourScheme::Custom)
        } else {
            Ok(ColourScheme::Default)
        }
    } else {
        Ok(ColourScheme::Default)
    }
}

fn get_retention(config: &ConfigV2) -> error::Result<u64> {
    const DEFAULT_RETENTION_MS: u64 = 600 * 1000; // Keep 10 minutes of data.

    if let Some(retention) = &config.general.args.retention {
        Ok(match retention {
            StringOrNum::String(s) => try_parse_ms(s)?,
            StringOrNum::Num(n) => *n,
        })
    } else {
        Ok(DEFAULT_RETENTION_MS)
    }
}

#[cfg(test)]
mod test {
    use clap::FromArgMatches;
    use toml_edit::de::from_str;

    use super::{config::ConfigV2, get_color_scheme, get_time_interval, get_widget_layout};
    use crate::{
        app::App,
        args::BottomArgs,
        canvas::styling::CanvasStyling,
        data_collection::temperature::TemperatureType,
        options::{
            get_default_time_value, get_retention, get_temperature, get_update_rate, try_parse_ms,
        },
    };

    fn config_from_args(args: Vec<&str>) -> ConfigV2 {
        let mut config = ConfigV2::default();
        let app = crate::args::build_cmd();
        let mut matches = app.get_matches_from(args);
        config.merge(BottomArgs::from_arg_matches_mut(&mut matches).unwrap());

        config
    }

    #[test]
    fn default_temp_is_celsius() {
        let config = config_from_args(vec!["btm"]);
        assert_eq!(get_temperature(&config), Ok(TemperatureType::Celsius));
    }

    #[test]
    fn can_set_temp_arg() {
        let config = config_from_args(vec!["btm", "-k"]);
        assert_eq!(get_temperature(&config), Ok(TemperatureType::Kelvin));
    }

    #[test]
    fn can_set_temp_cfg() {
        let config = from_str::<ConfigV2>("[temperature]\ntemperature_type='kelvin'").unwrap();
        assert_eq!(get_temperature(&config), Ok(TemperatureType::Kelvin));
    }

    #[test]
    fn skipped_temp_field_works() {
        let config = from_str::<ConfigV2>("[temperature]\nkelvin=true").unwrap();
        assert_eq!(get_temperature(&config), Ok(TemperatureType::Celsius));
    }

    #[test]
    fn verify_try_parse_ms() {
        let a = "100s";
        let b = "100";
        let c = "1 min";
        let d = "1 hour 1 min";

        assert_eq!(try_parse_ms(a), Ok(100 * 1000));
        assert_eq!(try_parse_ms(b), Ok(100));
        assert_eq!(try_parse_ms(c), Ok(60 * 1000));
        assert_eq!(try_parse_ms(d), Ok(3660 * 1000));

        let a_bad = "1 test";
        let b_bad = "-100";

        assert!(try_parse_ms(a_bad).is_err());
        assert!(try_parse_ms(b_bad).is_err());
    }

    #[test]
    fn matches_human_times_1() {
        let config = config_from_args(vec!["btm", "--time_delta", "2 min"]);

        assert_eq!(
            get_time_interval(&config, 60 * 60 * 1000),
            Ok(2 * 60 * 1000)
        );
    }

    #[test]
    fn matches_human_times_2() {
        let config = config_from_args(vec!["btm", "--default_time_value", "300s"]);

        assert_eq!(
            get_default_time_value(&config, 60 * 60 * 1000),
            Ok(5 * 60 * 1000)
        );
    }

    #[test]
    fn matches_number_times_1() {
        let config = config_from_args(vec!["btm", "--time_delta", "120000"]);

        assert_eq!(
            get_time_interval(&config, 60 * 60 * 1000),
            Ok(2 * 60 * 1000)
        );
    }

    #[test]
    fn matches_number_times_2() {
        let config = config_from_args(vec!["btm", "--default_time_value", "300000"]);

        assert_eq!(
            get_default_time_value(&config, 60 * 60 * 1000),
            Ok(5 * 60 * 1000)
        );
    }

    #[test]
    fn config_human_times() {
        let mut config = ConfigV2::default();
        config.general.args.time_delta = Some("2 min".into());
        config.general.args.default_time_value = Some("300s".into());
        config.general.args.rate = Some("1s".into());
        config.general.args.retention = Some("10m".into());

        assert_eq!(
            get_time_interval(&config, 60 * 60 * 1000),
            Ok(2 * 60 * 1000)
        );

        assert_eq!(
            get_default_time_value(&config, 60 * 60 * 1000),
            Ok(5 * 60 * 1000)
        );

        assert_eq!(get_update_rate(&config), Ok(1000));

        assert_eq!(get_retention(&config), Ok(600000));
    }

    #[test]
    fn config_number_times_as_string() {
        let mut config = ConfigV2::default();
        config.general.args.time_delta = Some(120000.into());
        config.general.args.default_time_value = Some(300000.into());
        config.general.args.rate = Some(1000.into());
        config.general.args.retention = Some(600000.into());

        assert_eq!(
            get_time_interval(&config, 60 * 60 * 1000),
            Ok(2 * 60 * 1000)
        );

        assert_eq!(
            get_default_time_value(&config, 60 * 60 * 1000),
            Ok(5 * 60 * 1000)
        );

        assert_eq!(get_update_rate(&config), Ok(1000));

        assert_eq!(get_retention(&config), Ok(600000));
    }

    #[test]
    fn config_number_times_as_num() {
        let mut config = ConfigV2::default();
        config.general.args.time_delta = Some(120000.into());
        config.general.args.default_time_value = Some(300000.into());
        config.general.args.rate = Some(1000.into());
        config.general.args.retention = Some(600000.into());

        assert_eq!(
            get_time_interval(&config, 60 * 60 * 1000),
            Ok(2 * 60 * 1000)
        );

        assert_eq!(
            get_default_time_value(&config, 60 * 60 * 1000),
            Ok(5 * 60 * 1000)
        );

        assert_eq!(get_update_rate(&config), Ok(1000));

        assert_eq!(get_retention(&config), Ok(600000));
    }

    fn create_app(config: ConfigV2) -> App {
        let (layout, id, ty) = get_widget_layout(&config).unwrap();
        let styling = CanvasStyling::new(get_color_scheme(&config).unwrap(), &config).unwrap();

        super::init_app(config, &layout, id, &ty, &styling).unwrap()
    }

    // TODO: There's probably a better way to create clap options AND unify together to avoid the possibility of
    // typos/mixing up. Use proc macros to unify on one struct?
    #[test]
    fn verify_cli_options_build() {
        let app = crate::args::build_cmd();

        let default_app = create_app(ConfigV2::default());

        // Skip battery since it's tricky to test depending on the platform/features we're testing with.
        let skip = ["help", "version", "celsius", "battery"];

        for arg in app.get_arguments().collect::<Vec<_>>() {
            let arg_name = arg
                .get_long_and_visible_aliases()
                .unwrap()
                .first()
                .unwrap()
                .to_owned();

            if !arg.get_action().takes_values() && !skip.contains(&arg_name) {
                let arg = format!("--{arg_name}");

                let arguments = vec!["btm", &arg];
                let config = config_from_args(arguments);

                let testing_app = create_app(config);

                if (default_app.app_config_fields == testing_app.app_config_fields)
                    && default_app.is_expanded == testing_app.is_expanded
                    && default_app
                        .states
                        .proc_state
                        .widget_states
                        .iter()
                        .zip(testing_app.states.proc_state.widget_states.iter())
                        .all(|(a, b)| (a.1.test_equality(b.1)))
                {
                    panic!("failed on {arg_name}");
                }
            }
        }
    }
}
