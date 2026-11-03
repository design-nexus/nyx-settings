//! Native account management in Settings.
use crate::widgets::Page;
pub fn build(page: &Page) {
    if super::vpn_accounts::adding() {
        super::vpn_accounts::add_page(page);
        return;
    }
    super::native_accounts::build(page);
    if super::native_accounts::on_list() {
        super::vpn_accounts::build(page);
    }
}
