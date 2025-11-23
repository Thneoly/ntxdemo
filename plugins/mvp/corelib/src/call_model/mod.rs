use crate::CoreLib;
use crate::exports::ntx::runner::core_call_model::Guest;
use crate::exports::ntx::runner::core_call_model::UserState;

pub struct CallModel;
pub struct CallModelUserState {}

impl Guest for CoreLib {
    fn step(_tick: u64, _desired_online: u32) -> Vec<UserState> {
        return vec![];
    }
}
