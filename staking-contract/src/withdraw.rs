use crate::*;
use near_sdk::{env, near, require, NearToken, Promise};

#[near]
impl Contract {
    /// Withdraw NEAR that has been credited to `withdrawable_balance` after epoch withdraw completes.
    #[payable]
    pub fn withdraw(&mut self, amount: Option<NearToken>) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();

        let pred = env::predecessor_account_id();
        let mut acc = self.accounts.get(&pred).expect("No account");
        let bal = acc.withdrawable_balance.as_yoctonear();
        let withdraw_yocto = match amount {
            Some(a) => {
                require!(a.as_yoctonear() <= bal, "Too much");
                a.as_yoctonear()
            }
            None => bal,
        };
        require!(withdraw_yocto > 0, "Nothing to withdraw");

        acc.withdrawable_balance = NearToken::from_yoctonear(bal - withdraw_yocto);
        self.accounts.insert(pred.clone(), acc);

        Promise::new(pred).transfer(NearToken::from_yoctonear(withdraw_yocto))
    }
}
