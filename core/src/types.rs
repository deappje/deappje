//! The object types (`docs/01-deappje/06-object-types.md`), one body
//! message each, with the invariants the plan names. Field numbers start
//! at 10; the common fields are [`Meta`](crate::schema::Meta). Every type
//! is at schema version 1.

use uuid::Uuid;

use crate::schema::{Invalid, Object, ObjectType};
use crate::{message, wire_enum};

/// The most an avatar or a thumbnail may be: it travels inside the value.
pub const MAX_INLINE_IMAGE: usize = 64 * 1024;

fn inline_image(bytes: &[u8], what: &'static str) -> Result<(), Invalid> {
    if bytes.len() > MAX_INLINE_IMAGE { Err(Invalid(what)) } else { Ok(()) }
}

fn non_empty(s: &str, what: &'static str) -> Result<(), Invalid> {
    if s.trim().is_empty() { Err(Invalid(what)) } else { Ok(()) }
}

fn currency(code: &str) -> Result<(), Invalid> {
    if code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()) { Ok(()) } else { Err(Invalid("the currency is not a three-letter code")) }
}

message! {
    /// A reference to a file in the environment's bucket, inside the
    /// owning object's value: the bucket key is derived from the object id
    /// and the slot, and the server never sees this.
    FileRef {
        /// The slot (`full`, `file`).
        1 => slot: String,
        /// SHA-256 of the encrypted bytes as uploaded.
        2 => sha256: Vec<u8>,
        /// The encrypted size in bytes.
        3 => size: i64,
        /// The blob key, 32 random bytes per file.
        4 => content_key: Vec<u8>,
    }
}

impl FileRef {
    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.slot, "a file needs a slot")?;
        if self.sha256.len() != 32 {
            return Err(Invalid("a file's hash is 32 bytes"));
        }
        if self.content_key.len() != 32 {
            return Err(Invalid("a file's key is 32 bytes"));
        }
        if self.size <= 0 {
            return Err(Invalid("a file has a size"));
        }
        Ok(())
    }
}

message! {
    /// A position on the map.
    Geo {
        /// Latitude, degrees.
        1 => lat: f64,
        /// Longitude, degrees.
        2 => lng: f64,
    }
}

impl Geo {
    fn validate(&self) -> Result<(), Invalid> {
        if (-90.0..=90.0).contains(&self.lat) && (-180.0..=180.0).contains(&self.lng) { Ok(()) } else { Err(Invalid("the position is off the map")) }
    }
}

// ---- Group (1, 2, 3) ----

wire_enum! {
    /// How a subgroup renders.
    pub enum GroupKind {
        /// A full group: albums, todos, expenses.
        #[default]
        Group = 0,
        /// A side chat in the parent's chat list.
        Chat = 1,
    }
}

message! {
    /// The group's profile; one per group, `ids::group_profile`.
    GroupProfile {
        10 => name: String,
        11 => description: String,
        /// An image, at most 64 KiB.
        12 => avatar: Vec<u8>,
        /// The chat's auto-delete setting in months: absent is 12, 0 is off.
        13 => message_retention_months: Option<u32>,
        14 => kind: GroupKind,
    }
}

impl GroupProfile {
    /// The months a message is kept: 12 unless set, 0 for never deleted.
    pub fn retention_months(&self) -> u32 {
        self.message_retention_months.unwrap_or(12)
    }
}

impl Object for GroupProfile {
    const TYPE: ObjectType = ObjectType::GroupProfile;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.name, "the group needs a name")?;
        inline_image(&self.avatar, "the avatar is over 64 KiB")
    }
}

message! {
    /// A birthday; the year may be unknown.
    Birthday {
        1 => month: u32,
        2 => day: u32,
        3 => year: Option<u32>,
    }
}

impl Birthday {
    fn validate(&self) -> Result<(), Invalid> {
        if (1..=12).contains(&self.month) && (1..=31).contains(&self.day) { Ok(()) } else { Err(Invalid("the birthday is not a date")) }
    }
}

message! {
    /// A member's profile in a group; one per member, `ids::member_profile`.
    MemberProfile {
        10 => display_name: String,
        /// An image, at most 64 KiB.
        11 => avatar: Vec<u8>,
        12 => birthday: Option<Birthday>,
    }
}

impl Object for MemberProfile {
    const TYPE: ObjectType = ObjectType::MemberProfile;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.display_name, "the profile needs a name")?;
        inline_image(&self.avatar, "the avatar is over 64 KiB")?;
        self.birthday.as_ref().map_or(Ok(()), Birthday::validate)
    }
}

message! {
    /// An invite's secret, so any holder can verify a key request that came
    /// through the invite (`02-device.md`).
    InviteSecret {
        /// The invite row's id on the server.
        10 => invite_id: Uuid,
        /// The 16 bytes of the link's fragment.
        11 => secret: Vec<u8>,
    }
}

impl Object for InviteSecret {
    const TYPE: ObjectType = ObjectType::InviteSecret;

    fn validate(&self) -> Result<(), Invalid> {
        if self.secret.len() == 16 { Ok(()) } else { Err(Invalid("an invite secret is 16 bytes")) }
    }
}

// ---- Timeline (10-14) ----

message! {
    /// A timeline entry; photos and comments are children.
    Post {
        10 => text: String,
    }
}

impl Object for Post {
    const TYPE: ObjectType = ObjectType::Post;
}

message! {
    /// The one photo type everywhere, under a post, an album or a message.
    Photo {
        10 => width: u32,
        11 => height: u32,
        /// When it was taken, milliseconds since the epoch, if known.
        12 => taken_at: Option<i64>,
        13 => caption: String,
        /// The grid image at a fixed size, at most 64 KiB, inside the value.
        14 => thumbnail: Vec<u8>,
        /// The full-size file at the uploader's upload quality.
        15 => full: Option<FileRef>,
    }
}

impl Object for Photo {
    const TYPE: ObjectType = ObjectType::Photo;

    fn validate(&self) -> Result<(), Invalid> {
        if self.width == 0 || self.height == 0 {
            return Err(Invalid("a photo has a size"));
        }
        inline_image(&self.thumbnail, "the thumbnail is over 64 KiB")?;
        self.full.as_ref().map_or(Ok(()), FileRef::validate)
    }
}

message! {
    /// An album; its photos are children.
    Album {
        10 => title: String,
        11 => cover_photo_id: Option<Uuid>,
    }
}

impl Object for Album {
    const TYPE: ObjectType = ObjectType::Album;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.title, "the album needs a title")
    }
}

message! {
    /// A comment under anything.
    Comment {
        10 => text: String,
    }
}

impl Object for Comment {
    const TYPE: ObjectType = ObjectType::Comment;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.text, "the comment is empty")
    }
}

message! {
    /// A user's reaction on a target; one per user, `ids::reaction`.
    Reaction {
        10 => emoji: String,
    }
}

impl Object for Reaction {
    const TYPE: ObjectType = ObjectType::Reaction;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.emoji, "the reaction is empty")
    }
}

// ---- Todos (20, 21) ----

message! {
    /// A todo list; its items are children.
    TodoList {
        10 => title: String,
    }
}

impl Object for TodoList {
    const TYPE: ObjectType = ObjectType::TodoList;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.title, "the list needs a title")
    }
}

message! {
    /// An item of a todo list.
    TodoItem {
        10 => text: String,
        11 => done: bool,
        12 => assignee: Option<Uuid>,
        /// Due, milliseconds since the epoch.
        13 => due: Option<i64>,
        /// The fractional index that orders the list.
        14 => pos: f64,
    }
}

impl Object for TodoItem {
    const TYPE: ObjectType = ObjectType::TodoItem;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.text, "the item is empty")
    }
}

// ---- Notes (30, 31) ----

message! {
    /// A note's title; the body is the combination of its `note_update`
    /// children.
    Note {
        10 => title: String,
    }
}

impl Object for Note {
    const TYPE: ObjectType = ObjectType::Note;
}

message! {
    /// One `yrs` update of a note: what one editing session on one device
    /// produced. Never edited.
    NoteUpdate {
        10 => update: Vec<u8>,
        /// The device's own count of its sessions on the note, a hint for
        /// ordering in the timeline; the CRDT needs no order.
        11 => seq_hint: i64,
    }
}

impl Object for NoteUpdate {
    const TYPE: ObjectType = ObjectType::NoteUpdate;

    fn validate(&self) -> Result<(), Invalid> {
        if self.update.is_empty() { Err(Invalid("the update is empty")) } else { Ok(()) }
    }
}

// ---- Expenses (40-43) ----

wire_enum! {
    /// How an expense is split among its participants.
    pub enum Split {
        /// One share each.
        #[default]
        Equal = 0,
        /// By each participant's weight.
        Weights = 1,
        /// Each participant's exact amount; they sum to the amount.
        Exact = 2,
    }
}

wire_enum! {
    /// What an expense is.
    pub enum ExpenseKind {
        /// An ordinary expense.
        #[default]
        Expense = 0,
        /// Tickets: an equal split, one share per participant, each with a
        /// `ticket_share` child.
        Ticket = 1,
    }
}

message! {
    /// A participant of an expense: who, and the weight or the exact amount
    /// in minor units when the split needs one.
    Participant, key 1 {
        1 => user_id: Uuid,
        2 => share: i64,
    }
}

message! {
    /// An expense in a pool; shares and balances are derived.
    Expense {
        /// In minor units of the currency.
        10 => amount: i64,
        /// ISO 4217, three uppercase letters.
        11 => currency: String,
        12 => paid_by: Uuid,
        13 => participants: Vec<Participant>,
        14 => split: Split,
        15 => kind: ExpenseKind,
        16 => description: String,
        /// The day, milliseconds since the epoch at midnight UTC.
        17 => date: i64,
    }
}

impl Object for Expense {
    const TYPE: ObjectType = ObjectType::Expense;

    fn validate(&self) -> Result<(), Invalid> {
        if self.amount <= 0 {
            return Err(Invalid("the amount is not positive"));
        }
        currency(&self.currency)?;
        if self.participants.is_empty() {
            return Err(Invalid("an expense needs a participant"));
        }
        let mut ids: Vec<Uuid> = self.participants.iter().map(|p| p.user_id).collect();
        ids.sort();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(Invalid("a participant is listed twice"));
        }
        match self.split {
            Split::Equal => {}
            Split::Weights => {
                if self.participants.iter().any(|p| p.share <= 0) {
                    return Err(Invalid("every weight is positive"));
                }
            }
            Split::Exact => {
                if self.participants.iter().any(|p| p.share < 0) {
                    return Err(Invalid("an exact share is not negative"));
                }
                let sum: i64 = self.participants.iter().map(|p| p.share).sum();
                if sum != self.amount {
                    return Err(Invalid("the exact shares do not sum to the amount"));
                }
            }
        }
        if self.kind == ExpenseKind::Ticket && self.split != Split::Equal {
            return Err(Invalid("tickets split equally"));
        }
        Ok(())
    }
}

message! {
    /// A payment made outside the app; money never moves here. Undo is a
    /// delete.
    Settlement {
        10 => from: Uuid,
        11 => to: Uuid,
        /// In minor units of the currency.
        12 => amount: i64,
        13 => currency: String,
        /// The day, milliseconds since the epoch at midnight UTC.
        14 => date: i64,
    }
}

impl Object for Settlement {
    const TYPE: ObjectType = ObjectType::Settlement;

    fn validate(&self) -> Result<(), Invalid> {
        if self.amount <= 0 {
            return Err(Invalid("the amount is not positive"));
        }
        currency(&self.currency)?;
        if self.from == self.to { Err(Invalid("a settlement is between two people")) } else { Ok(()) }
    }
}

wire_enum! {
    /// Where a ticket share stands.
    pub enum ShareStatus {
        /// With its holder.
        #[default]
        Held = 0,
        /// Offered to someone else.
        Offered = 1,
        /// Used at the gate.
        Used = 2,
    }
}

message! {
    /// The ticket file for one participant of an expense. A transfer is a
    /// holder change plus the matching participant change on the expense.
    TicketShare {
        /// The holder, when a member.
        10 => holder_user_id: Option<Uuid>,
        /// The holder's name, for a plus-one who is not in the app.
        11 => holder_name: String,
        12 => status: ShareStatus,
        /// The ticket file.
        13 => file: Option<FileRef>,
    }
}

impl Object for TicketShare {
    const TYPE: ObjectType = ObjectType::TicketShare;

    fn validate(&self) -> Result<(), Invalid> {
        if self.holder_user_id.is_none() && self.holder_name.trim().is_empty() {
            return Err(Invalid("a share has a holder"));
        }
        self.file.as_ref().map_or(Ok(()), FileRef::validate)
    }
}

message! {
    /// A pool of expenses settled independently; the group's general pool
    /// is `ids::general_pool`.
    ExpensePool {
        10 => title: String,
        11 => archived: bool,
    }
}

impl Object for ExpensePool {
    const TYPE: ObjectType = ObjectType::ExpensePool;
}

// ---- Calendar (50) ----

message! {
    /// A calendar event. A birthday for someone not in the app is a yearly
    /// all-day event.
    Event {
        10 => title: String,
        /// Start, milliseconds since the epoch.
        11 => starts: i64,
        /// End, milliseconds since the epoch, not before the start.
        12 => ends: i64,
        13 => all_day: bool,
        14 => location: String,
        15 => geo: Option<Geo>,
        16 => yearly: bool,
    }
}

impl Object for Event {
    const TYPE: ObjectType = ObjectType::Event;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.title, "the event needs a title")?;
        if self.ends < self.starts {
            return Err(Invalid("the event ends before it starts"));
        }
        self.geo.as_ref().map_or(Ok(()), Geo::validate)
    }
}

// ---- Polls (60, 61) ----

wire_enum! {
    /// What a poll asks.
    pub enum PollKind {
        /// A choice among options.
        #[default]
        Choice = 0,
        /// A date: the datepicker, every option a slot.
        Dates = 1,
    }
}

message! {
    /// An option of a poll.
    PollOption, key 1 {
        1 => id: Uuid,
        /// The text, for a choice.
        2 => text: String,
        /// The slot, milliseconds since the epoch, for dates.
        3 => slot: Option<i64>,
        /// The fractional index that orders the options.
        4 => pos: f64,
    }
}

message! {
    /// A poll; votes are children, so an option change never fights a vote.
    Poll {
        10 => question: String,
        11 => kind: PollKind,
        12 => options: Vec<PollOption>,
        /// Whether several answers may be yes.
        13 => multi: bool,
        /// When it closes, milliseconds since the epoch, if it does.
        14 => closes_at: Option<i64>,
    }
}

impl Object for Poll {
    const TYPE: ObjectType = ObjectType::Poll;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.question, "the poll needs a question")?;
        if self.options.is_empty() {
            return Err(Invalid("a poll needs an option"));
        }
        let mut ids: Vec<Uuid> = self.options.iter().map(|o| o.id).collect();
        ids.sort();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(Invalid("an option is listed twice"));
        }
        for o in &self.options {
            match self.kind {
                PollKind::Choice => non_empty(&o.text, "an option is empty")?,
                PollKind::Dates => {
                    if o.slot.is_none() {
                        return Err(Invalid("a date option needs a slot"));
                    }
                }
            }
        }
        Ok(())
    }
}

wire_enum! {
    /// An answer on one option.
    pub enum AnswerKind {
        #[default]
        No = 0,
        Yes = 1,
        Maybe = 2,
    }
}

message! {
    /// The answer on one option.
    Answer, key 1 {
        1 => option_id: Uuid,
        2 => answer: AnswerKind,
    }
}

message! {
    /// A user's answers on a poll; one per user, `ids::vote`.
    Vote {
        10 => answers: Vec<Answer>,
    }
}

impl Object for Vote {
    const TYPE: ObjectType = ObjectType::Vote;

    fn validate(&self) -> Result<(), Invalid> {
        let mut ids: Vec<Uuid> = self.answers.iter().map(|a| a.option_id).collect();
        ids.sort();
        if ids.windows(2).any(|w| w[0] == w[1]) { Err(Invalid("an option is answered twice")) } else { Ok(()) }
    }
}

// ---- Chat (70) ----

message! {
    /// A chat message; a reply in a thread has the thread's first message
    /// as its parent. An edit is a new seq, deletion a tombstone.
    Message {
        10 => text: String,
        /// The message quoted, if any.
        11 => reply_to: Option<Uuid>,
    }
}

impl Object for Message {
    const TYPE: ObjectType = ObjectType::Message;
}

// ---- Venues (80) ----

wire_enum! {
    /// What kind of place a venue is.
    pub enum VenueCategory {
        #[default]
        Other = 0,
        Bar = 1,
        Pub = 2,
        Club = 3,
        Restaurant = 4,
    }
}

message! {
    /// A venue recommendation; comments and reactions are children.
    Venue {
        10 => name: String,
        11 => category: VenueCategory,
        12 => address: String,
        13 => geo: Option<Geo>,
        14 => url: String,
        15 => notes: String,
    }
}

impl Object for Venue {
    const TYPE: ObjectType = ObjectType::Venue;

    fn validate(&self) -> Result<(), Invalid> {
        non_empty(&self.name, "the venue needs a name")?;
        self.geo.as_ref().map_or(Ok(()), Geo::validate)
    }
}

impl ObjectType {
    /// The schema of the type, for the rebase and the version check.
    pub fn schema(self) -> &'static crate::schema::Schema {
        match self {
            ObjectType::GroupProfile => &GroupProfile::SCHEMA,
            ObjectType::MemberProfile => &MemberProfile::SCHEMA,
            ObjectType::InviteSecret => &InviteSecret::SCHEMA,
            ObjectType::Post => &Post::SCHEMA,
            ObjectType::Photo => &Photo::SCHEMA,
            ObjectType::Album => &Album::SCHEMA,
            ObjectType::Comment => &Comment::SCHEMA,
            ObjectType::Reaction => &Reaction::SCHEMA,
            ObjectType::TodoList => &TodoList::SCHEMA,
            ObjectType::TodoItem => &TodoItem::SCHEMA,
            ObjectType::Note => &Note::SCHEMA,
            ObjectType::NoteUpdate => &NoteUpdate::SCHEMA,
            ObjectType::Expense => &Expense::SCHEMA,
            ObjectType::Settlement => &Settlement::SCHEMA,
            ObjectType::TicketShare => &TicketShare::SCHEMA,
            ObjectType::ExpensePool => &ExpensePool::SCHEMA,
            ObjectType::Event => &Event::SCHEMA,
            ObjectType::Poll => &Poll::SCHEMA,
            ObjectType::Vote => &Vote::SCHEMA,
            ObjectType::Message => &Message::SCHEMA,
            ObjectType::Venue => &Venue::SCHEMA,
        }
    }

    /// Validates a plaintext value of the type: decodes it as its document
    /// and checks the invariants. What the rebase result goes through
    /// before it is pushed.
    pub fn validate_value(self, plaintext: &[u8]) -> Result<(), ValueError> {
        fn check<T: Object>(plaintext: &[u8]) -> Result<(), ValueError> {
            crate::schema::Document::<T>::decode(plaintext)?.validate().map_err(ValueError::Invalid)
        }
        match self {
            ObjectType::GroupProfile => check::<GroupProfile>(plaintext),
            ObjectType::MemberProfile => check::<MemberProfile>(plaintext),
            ObjectType::InviteSecret => check::<InviteSecret>(plaintext),
            ObjectType::Post => check::<Post>(plaintext),
            ObjectType::Photo => check::<Photo>(plaintext),
            ObjectType::Album => check::<Album>(plaintext),
            ObjectType::Comment => check::<Comment>(plaintext),
            ObjectType::Reaction => check::<Reaction>(plaintext),
            ObjectType::TodoList => check::<TodoList>(plaintext),
            ObjectType::TodoItem => check::<TodoItem>(plaintext),
            ObjectType::Note => check::<Note>(plaintext),
            ObjectType::NoteUpdate => check::<NoteUpdate>(plaintext),
            ObjectType::Expense => check::<Expense>(plaintext),
            ObjectType::Settlement => check::<Settlement>(plaintext),
            ObjectType::TicketShare => check::<TicketShare>(plaintext),
            ObjectType::ExpensePool => check::<ExpensePool>(plaintext),
            ObjectType::Event => check::<Event>(plaintext),
            ObjectType::Poll => check::<Poll>(plaintext),
            ObjectType::Vote => check::<Vote>(plaintext),
            ObjectType::Message => check::<Message>(plaintext),
            ObjectType::Venue => check::<Venue>(plaintext),
        }
    }
}

/// Why a plaintext value is not a valid object of its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueError {
    /// Not a value of the type at all.
    Schema(crate::schema::SchemaError),
    /// A value of the type that breaks an invariant.
    Invalid(Invalid),
}

impl std::fmt::Display for ValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueError::Schema(e) => write!(f, "{e}"),
            ValueError::Invalid(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ValueError {}

impl From<crate::schema::SchemaError> for ValueError {
    fn from(e: crate::schema::SchemaError) -> Self {
        ValueError::Schema(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Document, Message as _};

    fn user(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn every_type_has_a_schema_whose_fields_start_at_ten_and_ascend() {
        for t in ObjectType::ALL {
            let s = t.schema();
            assert_eq!(s.version, 1, "{}", t.name());
            assert!(s.fields.iter().all(|f| f.number >= 10), "{}", t.name());
            assert!(s.fields.windows(2).all(|w| w[0].number < w[1].number), "{}", t.name());
        }
    }

    #[test]
    fn the_invariants_the_plan_names() {
        let ok = Event { title: "Dinner".into(), starts: 10, ends: 20, ..Event::default() };
        assert!(ok.validate().is_ok());
        assert_eq!(Event { ends: 5, ..ok.clone() }.validate(), Err(Invalid("the event ends before it starts")));
        assert_eq!(Event { geo: Some(Geo { lat: 91.0, lng: 0.0 }), ..ok }.validate(), Err(Invalid("the position is off the map")));

        let dinner = Expense {
            amount: 3000,
            currency: "EUR".into(),
            paid_by: user(1),
            participants: vec![Participant { user_id: user(1), share: 0 }, Participant { user_id: user(2), share: 0 }],
            description: "dinner".into(),
            date: 1,
            ..Expense::default()
        };
        assert!(dinner.validate().is_ok());
        assert_eq!(Expense { currency: "eur".into(), ..dinner.clone() }.validate(), Err(Invalid("the currency is not a three-letter code")));
        assert_eq!(Expense { split: Split::Weights, ..dinner.clone() }.validate(), Err(Invalid("every weight is positive")));
        let exact = Expense { split: Split::Exact, participants: vec![Participant { user_id: user(1), share: 1000 }, Participant { user_id: user(2), share: 2000 }], ..dinner.clone() };
        assert!(exact.validate().is_ok());
        assert_eq!(Expense { amount: 2999, ..exact }.validate(), Err(Invalid("the exact shares do not sum to the amount")));
        assert_eq!(Expense { participants: vec![Participant { user_id: user(1), share: 0 }; 2], ..dinner.clone() }.validate(), Err(Invalid("a participant is listed twice")));
        let weighted = vec![Participant { user_id: user(1), share: 1 }, Participant { user_id: user(2), share: 2 }];
        assert_eq!(Expense { kind: ExpenseKind::Ticket, split: Split::Weights, participants: weighted, ..dinner }.validate(), Err(Invalid("tickets split equally")));

        let poll = Poll { question: "When?".into(), kind: PollKind::Dates, options: vec![PollOption { id: user(1), slot: Some(5), ..PollOption::default() }], ..Poll::default() };
        assert!(poll.validate().is_ok());
        assert_eq!(Poll { options: vec![PollOption { id: user(1), ..PollOption::default() }], ..poll.clone() }.validate(), Err(Invalid("a date option needs a slot")));
        assert_eq!(Poll { kind: PollKind::Choice, ..poll }.validate(), Err(Invalid("an option is empty")));

        assert_eq!(GroupProfile { name: "Us".into(), avatar: vec![0; MAX_INLINE_IMAGE + 1], ..GroupProfile::default() }.validate(), Err(Invalid("the avatar is over 64 KiB")));
        assert_eq!(GroupProfile { name: "Us".into(), ..GroupProfile::default() }.retention_months(), 12);
        assert_eq!(GroupProfile { name: "Us".into(), message_retention_months: Some(0), ..GroupProfile::default() }.retention_months(), 0);
        assert_eq!(MemberProfile { display_name: "Anna".into(), birthday: Some(Birthday { month: 13, day: 1, year: None }), ..MemberProfile::default() }.validate(), Err(Invalid("the birthday is not a date")));
        assert_eq!(Settlement { from: user(1), to: user(1), amount: 5, currency: "EUR".into(), date: 1 }.validate(), Err(Invalid("a settlement is between two people")));
        assert_eq!(TicketShare::default().validate(), Err(Invalid("a share has a holder")));
        assert!(TicketShare { holder_name: "Bob's cousin".into(), ..TicketShare::default() }.validate().is_ok());
    }

    #[test]
    fn validate_value_runs_the_document_through_its_type() {
        let doc = Document::child_of(user(9), user(1), 1_000, Comment { text: "nice".into() });
        assert!(ObjectType::Comment.validate_value(&doc.encode()).is_ok());
        let root = Document::new(user(1), 1_000, Comment { text: "nice".into() });
        assert_eq!(ObjectType::Comment.validate_value(&root.encode()), Err(ValueError::Invalid(Invalid("this object needs a parent"))));
        assert!(matches!(ObjectType::Expense.validate_value(&[0x08]), Err(ValueError::Schema(_))));
        let retention = GroupProfile { name: "Us".into(), message_retention_months: Some(0), ..GroupProfile::default() };
        assert_eq!(GroupProfile::decode(&retention.encode()).unwrap(), retention, "Some(0) survives the wire");
    }
}
