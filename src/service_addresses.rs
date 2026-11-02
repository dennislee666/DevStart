//! Session-scoped HTTP addresses advertised by process output, matched to live listeners.
#[derive(Default)]
pub struct Addresses {
    candidates: Vec<(u16, String)>,
    remembered: Vec<(u16, String)>,
}
impl Addresses {
    pub fn observe(&mut self, line: &str) {
        for token in
            line.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '\"' | '\'' | '`'))
        {
            let token = token
                .trim_start_matches(['(', '='])
                .trim_end_matches([',', ';', '.', ')']);
            if let Some(address) = local_http_address(token)
                && !self.candidates.contains(&address)
                && self.candidates.len() < 16
            {
                self.candidates.push(address);
            }
        }
    }
    pub fn listening(&self, ports: &[u16]) -> Vec<String> {
        let current = self.candidates.iter().any(|(port, _)| ports.contains(port));
        let source = if current {
            &self.candidates
        } else {
            &self.remembered
        };
        source
            .iter()
            .filter(|(port, _)| ports.contains(port))
            .take(4)
            .map(|(_, url)| url.clone())
            .collect()
    }
}
fn local_http_address(value: &str) -> Option<(u16, String)> {
    let (scheme, rest) = value.split_once("://")?;
    if !matches!(scheme, "http" | "https") || value.len() > 2048 {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let (host, port) = if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, suffix) = ipv6.split_once(']')?;
        let port = if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':')?)
        };
        (host, port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        (host, Some(port))
    } else {
        (authority, None)
    };
    let port: u16 = match port {
        Some(p) => p.parse().ok()?,
        None => {
            if scheme == "https" {
                443
            } else {
                80
            }
        }
    };
    if port == 0 {
        return None;
    }
    let ip = host.parse::<std::net::IpAddr>().ok();
    let local = host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || ip.is_some_and(|ip| match ip {
            std::net::IpAddr::V4(ip) => {
                ip.is_loopback() || ip.is_private() || ip.is_unspecified() || ip.is_link_local()
            }
            std::net::IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
            }
        });
    if !local {
        return None;
    }
    let url = if ip.is_some_and(|ip| ip.is_unspecified()) {
        format!("{scheme}://localhost:{port}{}", &rest[end..])
    } else {
        value.to_owned()
    };
    Some((port, url))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exposes_only_advertised_local_addresses_on_live_ports() {
        let mut addresses = Addresses::default();
        for line in [
            "Local: http://localhost:3000/official/",
            "Local: http://localhost:3000/official/",
            "Network: http://192.168.1.2:3000/official/",
            "More info: https://sass-lang.com/d/legacy-js-api",
            "upstream http://localhost:9121/api",
            "http://user:secret@localhost:3000/",
        ] {
            addresses.observe(line);
        }
        assert!(addresses.listening(&[]).is_empty());
        assert_eq!(
            addresses.listening(&[3000]),
            [
                "http://localhost:3000/official/",
                "http://192.168.1.2:3000/official/"
            ]
        );
    }
    #[test]
    fn remembered_address_requires_live_port_and_fresh_output_replaces_it() {
        let mut addresses = Addresses::default();
        addresses
            .remembered
            .push((3000, "http://localhost:3000/old/".into()));
        assert!(addresses.listening(&[]).is_empty());
        assert!(addresses.listening(&[4000]).is_empty());
        assert_eq!(addresses.listening(&[3000]), ["http://localhost:3000/old/"]);
        addresses.observe("Local: http://localhost:3000/new/");
        assert_eq!(addresses.listening(&[3000]), ["http://localhost:3000/new/"]);
    }
    #[test]
    fn normalizes_wildcard_bind_addresses_and_retains_https_paths() {
        let mut addresses = Addresses::default();
        addresses.observe("http://0.0.0.0:8080/app/ https://[::]:8443/ui http://[::1]:8080/ https://localhost/path");
        assert_eq!(
            addresses.listening(&[8080, 8443, 443]),
            [
                "http://localhost:8080/app/",
                "https://localhost:8443/ui",
                "http://[::1]:8080/",
                "https://localhost/path"
            ]
        );
    }
}

/// Cache verified addresses outside launch configuration; display still requires a live listener.
#[derive(Default)]
pub struct Cache {
    loaded: bool,
    entries: std::collections::BTreeMap<String, Vec<String>>,
}
impl Cache {
    pub fn sync(
        &mut self,
        root: &std::path::Path,
        runs: &mut [crate::runtime::Run],
    ) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        let path = root.join("service-addresses.json");
        if !self.loaded {
            self.loaded = true;
            match std::fs::read(&path) {
                Ok(bytes) => {
                    self.entries = serde_json::from_slice(&bytes)
                        .map_err(|e| format!("读取访问地址缓存: {e}"))?
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("读取访问地址缓存: {e}")),
            }
        }
        let mut dirty = false;
        for run in runs
            .iter_mut()
            .filter(|r| !r.superseded && r.external.is_none())
        {
            let identity = serde_json::to_vec(&(run.script_id, &run.cwd, &run.command))
                .expect("string identity");
            let key = format!("{:x}", Sha256::digest(identity));
            run.addresses.remembered = self
                .entries
                .get(&key)
                .into_iter()
                .flatten()
                .filter_map(|url| local_http_address(url))
                .collect();
            if run.state != crate::runtime::State::Running {
                continue;
            }
            let fresh: Vec<_> = run
                .addresses
                .candidates
                .iter()
                .filter(|(port, _)| run.listening_ports.contains(port))
                .take(4)
                .map(|(_, url)| url.clone())
                .collect();
            if !fresh.is_empty() && self.entries.get(&key) != Some(&fresh) {
                self.entries.insert(key, fresh);
                dirty = true;
            }
        }
        if dirty {
            // Bound stale entries left by removed or reconfigured launch items.
            while self.entries.len() > 512 {
                self.entries.pop_first();
            }
            crate::store::atomic_write(
                &path,
                &serde_json::to_vec_pretty(&self.entries).map_err(|e| e.to_string())?,
            )?;
        }
        Ok(())
    }
}
