#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::ffi::OsStr;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Output;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Stdio;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::time::Duration;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use tokio::process::Command;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tokio::time::timeout;

use super::CheckStatus;
use super::DoctorCheck;
use super::DoctorIssue;

#[cfg(any(target_os = "macos", target_os = "windows"))]
const PRODUCT_QUERY_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(any(target_os = "macos", target_os = "windows"))]
const MAX_PRODUCT_OUTPUT_BYTES: usize = 64 * 1024;

pub(super) enum EndpointInspection {
    Complete(Vec<&'static str>),
    #[cfg(any(target_os = "windows", test))]
    Partial(Vec<&'static str>),
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    Unavailable,
}

pub(super) async fn check() -> DoctorCheck {
    endpoint_check(endpoint_products().await)
}

pub(super) fn endpoint_check(inspection: EndpointInspection) -> DoctorCheck {
    let product_name = codex_product_info::Product::current().short_display_name();
    let (products, visibility_incomplete) = match inspection {
        EndpointInspection::Complete(products) => (products, false),
        #[cfg(any(target_os = "windows", test))]
        EndpointInspection::Partial(products) => (products, true),
        #[cfg(any(target_os = "macos", target_os = "windows", test))]
        EndpointInspection::Unavailable => {
            return DoctorCheck::new(
                "security.endpoint",
                "security",
                CheckStatus::Warning,
                "endpoint protection inspection unavailable",
            )
            .detail("endpoint products: unavailable");
        }
    };

    if products.is_empty() {
        let (summary, detail) = if cfg!(any(target_os = "macos", target_os = "windows")) {
            ("no supported endpoint protection detected", "none detected")
        } else {
            (
                "endpoint protection is not inspected on this platform",
                "not inspected on this platform",
            )
        };
        return DoctorCheck::new("security.endpoint", "security", CheckStatus::Ok, summary)
            .detail(format!("endpoint products: {detail}"));
    }

    let targets = if cfg!(target_os = "windows") {
        format!(
            "signed {product_name} app; compatibility executables codex.exe, codex-windows-sandbox-setup.exe, codex-command-runner.exe, and codex-code-mode-host.exe"
        )
    } else if cfg!(target_os = "macos") {
        format!(
            "signing team 2DC432GLL2 plus the installed {product_name} app identity, signed agent, and required helpers"
        )
    } else {
        format!("verified {product_name} app, agent, and required helpers")
    };
    let mut check = DoctorCheck::new(
        "security.endpoint",
        "security",
        CheckStatus::Warning,
        format!("endpoint protection detected; {product_name} exclusions are unverified"),
    )
    .detail(format!("endpoint products: {}", products.join(", ")))
    .detail(format!("exclusion targets: {targets}"))
    .detail(format!("{product_name} exclusions: not verified"))
    .remediation(format!("ask your security administrator to verify {product_name} exclusions and required helper allowances"));
    if visibility_incomplete {
        check = check.detail("additional endpoint products: unavailable");
    }

    for product in products {
        let remedy = match product {
            "CrowdStrike Falcon" => {
                format!(
                    "CrowdStrike Falcon: Add a certificate or IOA exclusion for {product_name}. If sensor overhead continues, exclude the {product_name} agent from sensor visibility. Keep monitoring descendant processes."
                )
            }
            "BeyondTrust Privilege Management" => {
                format!(
                    "BeyondTrust: Remove {product_name} from application blocking rules. Add allow rules for {product_name} helper executables. Do not grant administrator privileges."
                )
            }
            "Microsoft Defender" => {
                format!(
                    "Microsoft Defender: Add a certificate or executable-path exclusion for {product_name} and its helpers. If Attack Surface Reduction blocks {product_name}, add a rule exclusion. If Controlled Folder Access blocks {product_name}, allow the app."
                )
            }
            "SentinelOne" => {
                format!(
                    "SentinelOne: Add a signer, file-hash, or executable-path exclusion for {product_name} and its helpers."
                )
            }
            "Jamf Protect" => {
                format!(
                    "Jamf Protect: Add an Override Threat Prevention exception for the {product_name} app and helper signing identities. If analytics cause delays, add an Ignore System Events for Analytics exception."
                )
            }
            _ => {
                format!(
                    "Add an exclusion for {product_name} and its helpers. Use the endpoint product instructions."
                )
            }
        };
        check = check.issue(
            DoctorIssue::new(
                CheckStatus::Warning,
                format!(
                    "{product} can interfere with {product_name}. Verify {product_name} exclusions."
                ),
            )
            .measured("not verified")
            .expected(format!("{product_name} application and helper exclusions"))
            .remedy(remedy)
            .field(format!("{product_name} exclusions")),
        );
    }

    check
}

async fn endpoint_products() -> EndpointInspection {
    #[cfg(target_os = "windows")]
    {
        let system32 = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32");
        let service = system32.join("sc.exe");
        let (crowdstrike, beyondtrust, defender, sentinelone) = tokio::join!(
            product_command(&service, &["query", "CSFalconService"]),
            product_command(&service, &["query", "DefendpointService"]),
            product_command(&service, &["query", "WinDefend"]),
            product_command(&service, &["query", "SentinelAgent"]),
        );

        let mut products = Vec::new();
        let mut visibility_incomplete = false;
        for (product, output) in [
            ("CrowdStrike Falcon", crowdstrike),
            ("BeyondTrust Privilege Management", beyondtrust),
            ("Microsoft Defender", defender),
            ("SentinelOne", sentinelone),
        ] {
            match output {
                Some(output) if output.status.success() => {
                    if product != "Microsoft Defender"
                        || String::from_utf8_lossy(&output.stdout).lines().any(|line| {
                            line.split_once(':').is_some_and(|(_, value)| {
                                value.split_whitespace().next() == Some("4")
                            })
                        })
                    {
                        products.push(product);
                    }
                }
                Some(output) if output.status.code() == Some(1060) => {}
                Some(_) | None => visibility_incomplete = true,
            }
        }

        if visibility_incomplete {
            if products.is_empty() {
                EndpointInspection::Unavailable
            } else {
                EndpointInspection::Partial(products)
            }
        } else {
            EndpointInspection::Complete(products)
        }
    }

    #[cfg(target_os = "macos")]
    {
        const PRODUCTS: &[(&str, &str, &str)] = &[
            (
                "CrowdStrike Falcon",
                "X9E956P446",
                "com.crowdstrike.falcon.Agent",
            ),
            (
                "BeyondTrust Privilege Management",
                "2ZS8T6NYB8",
                "com.beyondtrust.endpointsecurity",
            ),
            (
                "Microsoft Defender",
                "UBF8T346G9",
                "com.microsoft.wdav.epsext",
            ),
            (
                "SentinelOne",
                "4AYE5J54KN",
                "com.sentinelone.network-monitoring",
            ),
            (
                "Jamf Protect",
                "483DWKW443",
                "com.jamf.protect.security-extension",
            ),
        ];
        let Some(output) = product_command("/usr/bin/systemextensionsctl", &["list"])
            .await
            .filter(|output| output.status.success())
        else {
            return EndpointInspection::Unavailable;
        };
        EndpointInspection::Complete(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let mut columns = line.split_whitespace();
                    if columns.next() != Some("*") || columns.next() != Some("*") {
                        return None;
                    }
                    let team = columns.next()?;
                    let bundle = columns.next()?;
                    PRODUCTS.iter().find_map(|&(name, signer, identifier)| {
                        (team == signer && bundle == identifier).then_some(name)
                    })
                })
                .collect(),
        )
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        EndpointInspection::Complete(Vec::new())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn product_command(program: impl AsRef<OsStr>, args: &[&str]) -> Option<Output> {
    let mut command = Command::from(codex_utils_process::background_command(program));
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    timeout(PRODUCT_QUERY_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()
        .filter(|output| output.stdout.len() <= MAX_PRODUCT_OUTPUT_BYTES)
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
