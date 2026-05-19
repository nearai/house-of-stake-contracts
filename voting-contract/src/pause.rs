use crate::*;
use near_sdk::{assert_one_yocto, near, require};

#[near]
impl Contract {
    /// Checks if the contract is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Pauses the contract.
    /// Can only be called by the guardian.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn pause(&mut self) {
        assert_one_yocto();
        self.assert_guardian();
        self.paused = true;
    }

    /// Unpauses the contract.
    /// Can only be called by the owner.
    /// Requires 1 yocto NEAR.
    #[payable]
    pub fn unpause(&mut self) {
        assert_one_yocto();
        self.assert_owner();
        self.paused = false;
    }
}

impl Contract {
    pub fn assert_not_paused(&self) {
        require!(!self.paused, "Contract is paused. Please try again later.");
    }
}

#[cfg(test)]
mod tests {
    use crate::test_utils::*;

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

}
