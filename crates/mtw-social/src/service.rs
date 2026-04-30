//! Local cache + publish pipeline for social events. The single mutation
//! codepath is [`SocialService::ingest`]; create-* helpers produce a signed
//! `MtwMessage`, ingest it locally, then return the message for the caller to
//! route through `mtw-router`/`mtw-federation`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use mtw_identity::{verify_message, MtwIdentity};
use mtw_protocol::MtwMessage;
use tracing::warn;

use crate::error::{Result, SocialError};
use crate::event::{
    Attachment, Delete, Follow, Post, Profile, Reaction, Reply, Repost, SocialEvent,
};

/// A post enriched with author + ordering metadata, the way feeds want it.
#[derive(Debug, Clone)]
pub struct AnnotatedPost {
    pub id: String,
    pub author: String,
    pub timestamp: u64,
    pub post: Post,
}

/// A reply enriched with author + ordering metadata.
#[derive(Debug, Clone)]
pub struct AnnotatedReply {
    pub id: String,
    pub author: String,
    pub timestamp: u64,
    pub reply: Reply,
}

/// Replaceable record: the timestamp lets us reject older versions that arrive
/// out of order.
#[derive(Debug, Clone)]
struct Versioned<T> {
    timestamp: u64,
    value: T,
}

#[derive(Default)]
struct State {
    /// pubkey → latest profile
    profiles: HashMap<String, Versioned<Profile>>,
    /// pubkey → latest follow list
    follows: HashMap<String, Versioned<HashSet<String>>>,
    /// message_id → annotated post
    posts: HashMap<String, AnnotatedPost>,
    /// message_id → annotated reply
    replies: HashMap<String, AnnotatedReply>,
    /// (target_id, reactor_pubkey) → reaction content (latest wins)
    reactions: HashMap<(String, String), Versioned<String>>,
    /// IDs the author asked to forget
    deleted: HashSet<String>,
}

pub struct SocialService {
    identity: Arc<MtwIdentity>,
    my_pubkey: String,
    state: Arc<RwLock<State>>,
}

impl SocialService {
    pub fn new(identity: Arc<MtwIdentity>) -> Self {
        let my_pubkey = identity.pubkey().to_hex();
        Self {
            identity,
            my_pubkey,
            state: Arc::new(RwLock::new(State::default())),
        }
    }

    /// Hex-encoded pubkey of the local identity.
    pub fn my_pubkey(&self) -> &str {
        &self.my_pubkey
    }

    // ── Publishing ────────────────────────────────────────────────────────

    pub fn create_post(
        &self,
        content: impl Into<String>,
        tags: Vec<String>,
        attachments: Vec<Attachment>,
    ) -> Result<MtwMessage> {
        let event = SocialEvent::Post(Post {
            content: content.into(),
            tags,
            mentions: Vec::new(),
            attachments,
        });
        self.publish(event)
    }

    pub fn update_profile(&self, profile: Profile) -> Result<MtwMessage> {
        self.publish(SocialEvent::Profile(profile))
    }

    pub fn set_follows(&self, follows: Vec<String>) -> Result<MtwMessage> {
        self.publish(SocialEvent::Follow(Follow { follows }))
    }

    pub fn react(
        &self,
        target_id: impl Into<String>,
        target_pubkey: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<MtwMessage> {
        self.publish(SocialEvent::Reaction(Reaction {
            target_id: target_id.into(),
            target_pubkey: target_pubkey.into(),
            content: content.into(),
        }))
    }

    pub fn repost(
        &self,
        target_id: impl Into<String>,
        target_pubkey: impl Into<String>,
        comment: Option<String>,
    ) -> Result<MtwMessage> {
        self.publish(SocialEvent::Repost(Repost {
            target_id: target_id.into(),
            target_pubkey: target_pubkey.into(),
            comment,
        }))
    }

    pub fn reply(
        &self,
        content: impl Into<String>,
        parent_id: impl Into<String>,
        parent_pubkey: impl Into<String>,
        root_id: Option<String>,
    ) -> Result<MtwMessage> {
        self.publish(SocialEvent::Reply(Reply {
            content: content.into(),
            parent_id: parent_id.into(),
            parent_pubkey: parent_pubkey.into(),
            root_id,
            mentions: Vec::new(),
        }))
    }

    pub fn delete(&self, target_ids: Vec<String>) -> Result<MtwMessage> {
        self.publish(SocialEvent::Delete(Delete { target_ids }))
    }

    fn publish(&self, event: SocialEvent) -> Result<MtwMessage> {
        let msg = event.into_message(&self.identity)?;
        // Trust the local message; it's freshly signed by us. Use the same
        // ingest path as remote messages so behaviour stays consistent.
        self.apply(&msg)?;
        Ok(msg)
    }

    // ── Ingestion ─────────────────────────────────────────────────────────

    /// Verify the signature on a message and apply it to local state.
    /// Idempotent: applying the same event twice is a no-op.
    pub fn ingest(&self, msg: &MtwMessage) -> Result<()> {
        let signed = verify_message(msg).map_err(|_| SocialError::BadSignature)?;
        if !signed {
            return Err(SocialError::Unsigned);
        }
        self.apply(msg)
    }

    fn apply(&self, msg: &MtwMessage) -> Result<()> {
        let sender = msg.pubkey.clone().ok_or(SocialError::NoSender)?;
        let event = SocialEvent::from_message(msg)?;
        let mut st = self.state.write().expect("social state poisoned");

        // Drop anything the author already deleted.
        if st.deleted.contains(&msg.id) {
            return Ok(());
        }

        match event {
            SocialEvent::Profile(profile) => {
                upsert_versioned(&mut st.profiles, sender, msg.timestamp, profile);
            }
            SocialEvent::Follow(follow) => {
                let set: HashSet<String> = follow.follows.into_iter().collect();
                upsert_versioned(&mut st.follows, sender, msg.timestamp, set);
            }
            SocialEvent::Post(post) => {
                st.posts.insert(
                    msg.id.clone(),
                    AnnotatedPost {
                        id: msg.id.clone(),
                        author: sender,
                        timestamp: msg.timestamp,
                        post,
                    },
                );
            }
            SocialEvent::Reply(reply) => {
                st.replies.insert(
                    msg.id.clone(),
                    AnnotatedReply {
                        id: msg.id.clone(),
                        author: sender,
                        timestamp: msg.timestamp,
                        reply,
                    },
                );
            }
            SocialEvent::Reaction(reaction) => {
                let key = (reaction.target_id.clone(), sender);
                let new = Versioned {
                    timestamp: msg.timestamp,
                    value: reaction.content,
                };
                match st.reactions.get(&key) {
                    // Only ignore if the existing one is strictly newer.
                    // Equal timestamps fall through to replace (matches local
                    // intent when two reactions are emitted within one ms).
                    Some(existing) if existing.timestamp > new.timestamp => {}
                    _ => {
                        st.reactions.insert(key, new);
                    }
                }
            }
            SocialEvent::Repost(_) => {
                // Reposts are append-only references; consumers can drive
                // them off `posts` + `replies` + an external index. Keeping
                // them out of state for v1 to avoid storing unbounded data.
            }
            SocialEvent::Delete(del) => {
                for id in &del.target_ids {
                    let authorized = st
                        .posts
                        .get(id)
                        .map(|p| p.author == sender)
                        .or_else(|| st.replies.get(id).map(|r| r.author == sender))
                        .unwrap_or(false);
                    if !authorized {
                        warn!(
                            target_id = %id,
                            requester = %sender,
                            "rejected delete: target absent or not owned by sender"
                        );
                        continue;
                    }
                    st.posts.remove(id);
                    st.replies.remove(id);
                    st.deleted.insert(id.clone());
                }
            }
        }
        Ok(())
    }

    // ── Queries ───────────────────────────────────────────────────────────

    pub fn profile_of(&self, pubkey: &str) -> Option<Profile> {
        self.state
            .read()
            .ok()
            .and_then(|s| s.profiles.get(pubkey).map(|v| v.value.clone()))
    }

    pub fn following(&self, pubkey: &str) -> Vec<String> {
        self.state
            .read()
            .ok()
            .and_then(|s| s.follows.get(pubkey).map(|v| v.value.iter().cloned().collect()))
            .unwrap_or_default()
    }

    /// Newest-first global timeline of posts (cap at `limit`).
    pub fn timeline(&self, limit: usize) -> Vec<AnnotatedPost> {
        let mut posts: Vec<AnnotatedPost> = match self.state.read() {
            Ok(s) => s.posts.values().cloned().collect(),
            Err(_) => return Vec::new(),
        };
        posts.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        posts.truncate(limit);
        posts
    }

    /// Newest-first timeline restricted to authors the local identity follows.
    pub fn home_timeline(&self, limit: usize) -> Vec<AnnotatedPost> {
        let following: HashSet<String> = self
            .state
            .read()
            .ok()
            .and_then(|s| s.follows.get(&self.my_pubkey).map(|v| v.value.clone()))
            .unwrap_or_default();
        let mut posts: Vec<AnnotatedPost> = match self.state.read() {
            Ok(s) => s
                .posts
                .values()
                .filter(|p| following.contains(&p.author) || p.author == self.my_pubkey)
                .cloned()
                .collect(),
            Err(_) => return Vec::new(),
        };
        posts.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        posts.truncate(limit);
        posts
    }

    /// `{reaction_content -> count}` for a given target message id.
    pub fn reactions_for(&self, target_id: &str) -> HashMap<String, u32> {
        let mut counts: HashMap<String, u32> = HashMap::new();
        if let Ok(s) = self.state.read() {
            for ((tid, _), v) in s.reactions.iter() {
                if tid == target_id {
                    *counts.entry(v.value.clone()).or_insert(0) += 1;
                }
            }
        }
        counts
    }

    /// Replies for a given parent id (newest first).
    pub fn replies_to(&self, parent_id: &str) -> Vec<AnnotatedReply> {
        let mut replies: Vec<AnnotatedReply> = match self.state.read() {
            Ok(s) => s
                .replies
                .values()
                .filter(|r| r.reply.parent_id == parent_id)
                .cloned()
                .collect(),
            Err(_) => return Vec::new(),
        };
        replies.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        replies
    }

    pub fn is_deleted(&self, id: &str) -> bool {
        self.state
            .read()
            .map(|s| s.deleted.contains(id))
            .unwrap_or(false)
    }
}

fn upsert_versioned<T>(
    map: &mut HashMap<String, Versioned<T>>,
    key: String,
    timestamp: u64,
    value: T,
) {
    match map.get(&key) {
        // Strict `>`: equal timestamps fall through to replace, so two
        // back-to-back local publishes within one ms behave as the user
        // intends. Out-of-order arrival from peers is still rejected when
        // the existing record is strictly newer.
        Some(existing) if existing.timestamp > timestamp => {}
        _ => {
            map.insert(key, Versioned { timestamp, value });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtw_protocol::Payload;

    fn svc() -> SocialService {
        let id = Arc::new(MtwIdentity::generate());
        SocialService::new(id)
    }

    #[test]
    fn post_roundtrip_through_local_ingest() {
        let s = svc();
        let msg = s.create_post("hola mundo", vec!["greeting".into()], vec![]).unwrap();
        assert!(msg.pubkey.is_some() && msg.sig.is_some());

        let timeline = s.timeline(10);
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].post.content, "hola mundo");
        assert_eq!(timeline[0].author, s.my_pubkey());
    }

    #[test]
    fn ingest_remote_post_after_signing_by_other_identity() {
        let alice = SocialService::new(Arc::new(MtwIdentity::generate()));
        let bob = SocialService::new(Arc::new(MtwIdentity::generate()));

        let alice_post = alice.create_post("from alice", vec![], vec![]).unwrap();
        bob.ingest(&alice_post).unwrap();
        let tl = bob.timeline(10);
        assert_eq!(tl.len(), 1);
        assert_eq!(tl[0].author, alice.my_pubkey());
    }

    #[test]
    fn unsigned_message_is_rejected() {
        let s = svc();
        let msg = mtw_protocol::MtwMessage::new(
            mtw_protocol::MsgType::Event,
            Payload::Json(serde_json::json!({"kind": "post", "content": "x"})),
        )
        .with_channel("social.posts");
        let err = s.ingest(&msg).unwrap_err();
        assert!(matches!(err, SocialError::Unsigned));
    }

    #[test]
    fn tampered_post_is_rejected() {
        let alice = SocialService::new(Arc::new(MtwIdentity::generate()));
        let bob = SocialService::new(Arc::new(MtwIdentity::generate()));

        let mut msg = alice.create_post("real", vec![], vec![]).unwrap();
        // Tamper with the payload after signing.
        msg.payload = Payload::Json(serde_json::json!({
            "kind": "post",
            "content": "fake"
        }));
        let err = bob.ingest(&msg).unwrap_err();
        assert!(matches!(err, SocialError::BadSignature));
    }

    #[test]
    fn profile_is_replaceable_by_newer_timestamp() {
        let s = svc();

        let mut older = s
            .update_profile(Profile {
                display_name: Some("v1".into()),
                ..Default::default()
            })
            .unwrap();
        // Publish "newer" with a later timestamp.
        let mut newer = s
            .update_profile(Profile {
                display_name: Some("v2".into()),
                ..Default::default()
            })
            .unwrap();

        // Make timestamps unambiguous regardless of system clock resolution.
        older.timestamp = 1;
        newer.timestamp = 2;
        // Re-sign after manual mutation so verify_message still passes.
        s.identity.sign_message(&mut older).unwrap();
        s.identity.sign_message(&mut newer).unwrap();

        let other = svc();
        other.ingest(&newer).unwrap();
        other.ingest(&older).unwrap(); // older arrives second — should NOT replace
        assert_eq!(
            other.profile_of(s.my_pubkey()).unwrap().display_name.unwrap(),
            "v2"
        );
    }

    #[test]
    fn follow_list_is_replaceable() {
        let s = svc();
        s.set_follows(vec!["a".into(), "b".into()]).unwrap();
        s.set_follows(vec!["c".into()]).unwrap();

        let f = s.following(s.my_pubkey());
        assert_eq!(f, vec!["c".to_string()]);
    }

    #[test]
    fn reactions_aggregate_by_content() {
        let alice = SocialService::new(Arc::new(MtwIdentity::generate()));
        let bob = SocialService::new(Arc::new(MtwIdentity::generate()));
        let carol = SocialService::new(Arc::new(MtwIdentity::generate()));

        let post = alice.create_post("hot take", vec![], vec![]).unwrap();
        bob.ingest(&post).unwrap();
        carol.ingest(&post).unwrap();

        let r1 = bob.react(&post.id, alice.my_pubkey(), "🔥").unwrap();
        let r2 = carol.react(&post.id, alice.my_pubkey(), "🔥").unwrap();
        let r3 = carol.react(&post.id, alice.my_pubkey(), "❤️").unwrap();

        // alice ingests everyone's reactions
        alice.ingest(&r1).unwrap();
        alice.ingest(&r2).unwrap();
        alice.ingest(&r3).unwrap();

        let counts = alice.reactions_for(&post.id);
        // carol's "❤️" replaces her earlier "🔥" (latest reaction per author)
        assert_eq!(counts.get("🔥"), Some(&1));
        assert_eq!(counts.get("❤️"), Some(&1));
    }

    #[test]
    fn delete_removes_own_post_only() {
        let alice = SocialService::new(Arc::new(MtwIdentity::generate()));
        let bob = SocialService::new(Arc::new(MtwIdentity::generate()));

        let alice_post = alice.create_post("regret it", vec![], vec![]).unwrap();
        bob.ingest(&alice_post).unwrap();
        assert_eq!(bob.timeline(10).len(), 1);

        // Bob tries to delete Alice's post — should be rejected silently.
        let bob_delete = bob.delete(vec![alice_post.id.clone()]).unwrap();
        alice.ingest(&bob_delete).unwrap();
        assert_eq!(alice.timeline(10).len(), 1, "bob cannot delete alice's post");

        // Alice deletes her own post — should propagate.
        let alice_delete = alice.delete(vec![alice_post.id.clone()]).unwrap();
        bob.ingest(&alice_delete).unwrap();
        assert_eq!(bob.timeline(10).len(), 0);
        assert!(bob.is_deleted(&alice_post.id));
    }

    #[test]
    fn home_timeline_filters_to_followed_authors() {
        let alice = SocialService::new(Arc::new(MtwIdentity::generate()));
        let bob = SocialService::new(Arc::new(MtwIdentity::generate()));
        let stranger = SocialService::new(Arc::new(MtwIdentity::generate()));

        // Alice follows Bob but not the stranger.
        alice.set_follows(vec![bob.my_pubkey().to_string()]).unwrap();

        let bob_post = bob.create_post("from bob", vec![], vec![]).unwrap();
        let stranger_post = stranger.create_post("from stranger", vec![], vec![]).unwrap();

        alice.ingest(&bob_post).unwrap();
        alice.ingest(&stranger_post).unwrap();

        // Global timeline sees both…
        assert_eq!(alice.timeline(10).len(), 2);
        // …but home timeline filters to followed authors (+ self).
        let home = alice.home_timeline(10);
        assert_eq!(home.len(), 1);
        assert_eq!(home[0].post.content, "from bob");
    }

    #[test]
    fn replies_are_threaded_under_parent() {
        let alice = SocialService::new(Arc::new(MtwIdentity::generate()));
        let bob = SocialService::new(Arc::new(MtwIdentity::generate()));

        let parent = alice.create_post("anyone there?", vec![], vec![]).unwrap();
        bob.ingest(&parent).unwrap();

        let reply = bob
            .reply("yes", &parent.id, alice.my_pubkey(), None)
            .unwrap();
        alice.ingest(&reply).unwrap();

        let replies = alice.replies_to(&parent.id);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].reply.content, "yes");
    }
}
