//! Unit tests for raw CDP session refusal rules (`page.newCDPSession()`,
//! ticket #751, ADR 0085).

use rstest::rstest;

use super::refused_command;

// Covers: story:5-106
// Covers: error:cdp_command_refused
#[rstest]
#[case::attach_to_target("Target.attachToTarget", false, true)]
#[case::attach_to_browser_target("Target.attachToBrowserTarget", false, true)]
#[case::set_auto_attach("Target.setAutoAttach", false, true)]
#[case::expose_devtools("Target.exposeDevToolsProtocol", false, true)]
#[case::close_target("Target.closeTarget", false, true)]
#[case::create_target("Target.createTarget", false, true)]
#[case::create_browser_context("Target.createBrowserContext", false, true)]
#[case::dispose_browser_context("Target.disposeBrowserContext", false, true)]
#[case::page_close("Page.close", false, true)]
#[case::browser_close_managed("Browser.close", false, true)]
#[case::browser_close_borrowed("Browser.close", true, true)]
#[case::browser_crash_borrowed("Browser.crash", true, true)]
#[case::browser_crash_gpu_borrowed("Browser.crashGpuProcess", true, true)]
#[case::cpu_throttling("Emulation.setCPUThrottlingRate", false, false)]
#[case::runtime_evaluate("Runtime.evaluate", false, false)]
#[case::runtime_evaluate_borrowed("Runtime.evaluate", true, false)]
#[case::network_enable("Network.enable", false, false)]
#[case::dom_get_document("DOM.getDocument", false, false)]
fn raw_cdp_command_refusal_rules(
    #[case] method: &str,
    #[case] borrowed: bool,
    #[case] refused: bool,
) {
    assert_eq!(refused_command(method, borrowed).is_some(), refused);
}
