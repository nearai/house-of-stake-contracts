use crate::unit_tests::test_utils::*;

#[test]
fn guardian_can_pause() {
    let mut contract = fresh_contract();
    set_ctx(guardian(), 1, TEST_NOW_NS);
    contract.pause();
    assert!(contract.is_paused());
}

#[test]
fn owner_can_pause() {
    let mut contract = fresh_contract();
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.pause();
    assert!(contract.is_paused());
}

#[test]
#[should_panic(expected = "Only the guardian can call this method")]
fn non_guardian_cannot_pause() {
    let mut contract = fresh_contract();
    set_ctx(acc("rando.test.near"), 1, TEST_NOW_NS);
    contract.pause();
}

#[test]
#[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
fn pause_requires_one_yocto() {
    let mut contract = fresh_contract();
    set_ctx(guardian(), 0, TEST_NOW_NS);
    contract.pause();
}

#[test]
fn owner_can_unpause() {
    let mut contract = fresh_contract();
    contract.paused = true;
    set_ctx(owner(), 1, TEST_NOW_NS);
    contract.unpause();
    assert!(!contract.is_paused());
}

#[test]
#[should_panic(expected = "Only the owner can call this method")]
fn guardian_cannot_unpause() {
    let mut contract = fresh_contract();
    contract.paused = true;
    set_ctx(guardian(), 1, TEST_NOW_NS);
    contract.unpause();
}

#[test]
#[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
fn unpause_requires_one_yocto() {
    let mut contract = fresh_contract();
    contract.paused = true;
    set_ctx(owner(), 0, TEST_NOW_NS);
    contract.unpause();
}
