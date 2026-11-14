//! Online Accounts: the accounts your apps use, storage-only cloud accounts and VPN
//! services on the left; the chosen account and the apps it feeds on the right.
use crate::widgets::Page;
pub fn build(page: &Page) {
    if super::vpn_accounts::adding() {
        super::vpn_accounts::add_page(page);
        return;
    }
    let pv = page.split();
    super::native_accounts::build_with_preview(page, &pv);
    super::vpn_accounts::build(page);
}
