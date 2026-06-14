use foundry::prelude::*;

#[derive(Clone, Copy, FoundryId)]
#[foundry(id = GuardId, rename_all = "snake_case")]
pub enum AuthGuard {
    Api,
}

#[derive(Clone, Copy, FoundryId)]
#[foundry(id = PermissionId)]
pub enum Ability {
    #[foundry(value = "dashboard:view")]
    DashboardView,
    #[foundry(value = "realtime:chat")]
    RealtimeChat,
}

#[derive(Clone, Copy, FoundryId)]
#[foundry(id = RouteId)]
pub enum Route {
    #[foundry(value = "health")]
    Health,
    #[foundry(value = "users.store")]
    UsersStore,
}

pub const MOBILE_RULE: ValidationRuleId = ValidationRuleId::new("mobile");
pub const PING_COMMAND: CommandId = CommandId::new("ping");
pub const HEARTBEAT_SCHEDULE: ScheduleId = ScheduleId::new("heartbeat");
pub const CHAT_CHANNEL: ChannelId = ChannelId::new("chat");
pub const ECHO_EVENT: ChannelEventId = ChannelEventId::new("echo");
