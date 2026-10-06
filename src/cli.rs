use std::path::PathBuf;

pub const DEFAULT_HTTP_PORT: u16 = 17841;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliOptions {
    pub data_dir: Option<PathBuf>,
    pub http_port: u16,
    pub peer_port: Option<u16>,
    pub instance_name: Option<String>,
    pub advertise_host: Option<String>,
    pub demo: bool,
    pub demo_world: String,
    pub demo_zone: String,
    pub help: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            data_dir: None,
            http_port: DEFAULT_HTTP_PORT,
            peer_port: None,
            instance_name: None,
            advertise_host: None,
            demo: false,
            demo_world: "Wizard City".into(),
            demo_zone: "The Commons".into(),
            help: false,
        }
    }
}

impl CliOptions {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        let mut demo_world_set = false;
        let mut demo_zone_set = false;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => options.help = true,
                "--demo" => options.demo = true,
                "--data-dir" => {
                    options.data_dir = Some(PathBuf::from(required_value(&mut args, &arg)?));
                }
                "--http-port" => {
                    options.http_port = parse_port(&required_value(&mut args, &arg)?, &arg)?;
                }
                "--peer-port" => {
                    options.peer_port = Some(parse_port(&required_value(&mut args, &arg)?, &arg)?);
                }
                "--instance-name" => {
                    let name = required_value(&mut args, &arg)?;
                    if name.trim().is_empty()
                        || name.len() > 40
                        || name.chars().any(char::is_control)
                    {
                        return Err("instance name must be 1–40 printable characters".into());
                    }
                    options.instance_name = Some(name);
                }
                "--advertise-host" => {
                    let host = required_value(&mut args, &arg)?;
                    if host.trim().is_empty() || host.len() > 253 || host.contains('/') {
                        return Err("advertise host must be a hostname or IP address".into());
                    }
                    options.advertise_host = Some(host);
                }
                "--demo-world" => {
                    options.demo_world = required_value(&mut args, &arg)?;
                    demo_world_set = true;
                }
                "--demo-zone" => {
                    options.demo_zone = required_value(&mut args, &arg)?;
                    demo_zone_set = true;
                }
                _ => return Err(format!("unknown option: {arg}")),
            }
        }
        if demo_world_set != demo_zone_set {
            return Err("--demo-world and --demo-zone must be provided together".into());
        }
        if demo_world_set {
            if options.demo_world.trim().is_empty()
                || options.demo_zone.trim().is_empty()
                || options.demo_world.len() > 80
                || options.demo_zone.len() > 120
            {
                return Err("demo world must be 1–80 and zone 1–120 characters".into());
            }
            options.demo = true;
        }
        Ok(options)
    }
}

fn required_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| format!("{option} requires a value"))
}

fn parse_port(value: &str, option: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| format!("{option} must be a port between 1024 and 65535"))?;
    if port < 1024 {
        return Err(format!("{option} must be a port between 1024 and 65535"));
    }
    Ok(port)
}

pub const HELP: &str = "WizRust101-OBS options:\n\
  --data-dir PATH          Isolated directory for this instance's config.json\n\
  --http-port PORT         Loopback OBS HTTP port (default 17841)\n\
  --peer-port PORT         Party listener port (default 17842)\n\
  --instance-name NAME     Label this app/window/tray instance\n\
  --advertise-host HOST    Host address embedded in invites (e.g. 127.0.0.1)\n\
  --demo                   Use controllable mock game state; do not discover logs\n\
  --demo-world WORLD       Initial mock world (implies --demo; requires zone)\n\
  --demo-zone ZONE         Initial mock zone (implies --demo; requires world)\n\
  --help                   Show this help\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_isolated_mock_instance_options() {
        let options = CliOptions::parse(
            [
                "--data-dir",
                "./party/B",
                "--http-port",
                "17843",
                "--peer-port",
                "17844",
                "--instance-name",
                "Beta",
                "--advertise-host",
                "127.0.0.1",
                "--demo-world",
                "Krokotopia",
                "--demo-zone",
                "The Oasis",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        assert_eq!(options.data_dir, Some(PathBuf::from("./party/B")));
        assert_eq!(options.http_port, 17843);
        assert_eq!(options.peer_port, Some(17844));
        assert_eq!(options.instance_name.as_deref(), Some("Beta"));
        assert_eq!(options.advertise_host.as_deref(), Some("127.0.0.1"));
        assert!(options.demo);
        assert_eq!(options.demo_world, "Krokotopia");
        assert_eq!(options.demo_zone, "The Oasis");
    }

    #[test]
    fn rejects_invalid_ports_and_unpaired_mock_location() {
        assert!(CliOptions::parse(["--http-port", "80"].map(str::to_owned)).is_err());
        assert!(CliOptions::parse(["--demo-world", "Celestia"].map(str::to_owned)).is_err());
        assert!(CliOptions::parse(["--unknown"].map(str::to_owned)).is_err());
    }
}
