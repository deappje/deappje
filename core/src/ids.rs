//! Object ids. A random id is minted from the core's generator (never from
//! the crate's own randomness, which the browser build cannot have); a
//! singleton's id is UUIDv5 of its parent and its role, so every device
//! mints the same one (`docs/01-deappje/06-object-types.md`).

use uuid::Uuid;

use crate::rng::CoreRng;

/// A fresh random id (UUIDv4 layout) from the generator.
pub fn random(rng: &mut CoreRng) -> Uuid {
    uuid::Builder::from_random_bytes(rng.bytes16()).into_uuid()
}

/// The group's profile: `UUIDv5(group_id, "profile")`.
pub fn group_profile(group_id: Uuid) -> Uuid {
    Uuid::new_v5(&group_id, b"profile")
}

/// A member's profile in a group: `UUIDv5(group_id, user_id)`.
pub fn member_profile(group_id: Uuid, user_id: Uuid) -> Uuid {
    Uuid::new_v5(&group_id, user_id.as_bytes())
}

/// The group's general expense pool: `UUIDv5(group_id, "pool")`.
pub fn general_pool(group_id: Uuid) -> Uuid {
    Uuid::new_v5(&group_id, b"pool")
}

/// A user's reaction on a target: `UUIDv5(parent_id, user_id)`.
pub fn reaction(parent_id: Uuid, user_id: Uuid) -> Uuid {
    Uuid::new_v5(&parent_id, user_id.as_bytes())
}

/// A user's vote on a poll: `UUIDv5(poll_id, user_id)`.
pub fn vote(poll_id: Uuid, user_id: Uuid) -> Uuid {
    Uuid::new_v5(&poll_id, user_id.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_ids_are_v4_and_distinct() {
        let mut rng = CoreRng::from_seed([3; 32]);
        let a = random(&mut rng);
        let b = random(&mut rng);
        assert_ne!(a, b);
        assert_eq!(a.get_version_num(), 4);
        assert_eq!(random(&mut CoreRng::from_seed([3; 32])), a, "the generator decides");
    }

    #[test]
    fn singletons_are_the_same_on_every_device() {
        let g = Uuid::from_u128(1);
        let u = Uuid::from_u128(2);
        assert_eq!(group_profile(g), group_profile(g));
        assert_ne!(group_profile(g), general_pool(g));
        assert_eq!(member_profile(g, u), member_profile(g, u));
        assert_ne!(member_profile(g, u), member_profile(g, Uuid::from_u128(3)));
        assert_eq!(reaction(g, u), member_profile(g, u), "the same derivation: parent and user");
        assert_eq!(vote(g, u).get_version_num(), 5);
    }
}
