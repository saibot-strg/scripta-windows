//! Naming, merging and reassigning transcript speakers, keeping the voice
//! store in step. A port of the speaker part of Scripta's `AppModel`.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde::Deserialize;

use crate::library::{Library, SpeakerRow};
use crate::model::SpeakerRole;
use crate::voices::{MatchState, VoiceStore};

/// One name given in the assignment dialog.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameAssignment {
    pub speaker_id: String,
    pub name: String,
    pub remember: bool,
}

pub struct Speakers<'a> {
    pub library: &'a mut Library,
    pub voices: &'a mut VoiceStore,
    /// "Stimmen wiedererkennen" is on.
    pub recognizes: bool,
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

impl Speakers<'_> {
    /// An empty name restores the default ("Ich", "Andere", "Sprecher 2").
    /// The name of another speaker in the same transcript merges both: the
    /// diarization split one person into several voices. With voice
    /// recognition on, a name also teaches that person's voice, unless
    /// `remember` is off (a guest).
    pub fn rename(&mut self, speaker_id: &str, name: &str, remember: bool) -> Result<()> {
        let Some(speaker) = self.library.speaker(speaker_id)? else { return Ok(()) };
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            let existing = self
                .library
                .speakers(&speaker.transcript_id)?
                .into_iter()
                .find(|s| s.id != speaker.id && key(&s.display_name()) == key(trimmed));
            if let Some(existing) = existing {
                return self.merge(&speaker.id, &existing.id);
            }
        }
        self.set_name(&speaker, trimmed, remember)
    }

    /// Applies the names of the assignment dialog together. Speakers that end
    /// up with the same name are merged; a name that only matches another
    /// speaker's *old* name is not (e.g. two guessed names swapped).
    pub fn assign_names(&mut self, transcript_id: &str, assignments: &[NameAssignment]) -> Result<()> {
        let speakers = self.library.speakers(transcript_id)?;
        let mut final_names: HashMap<String, String> =
            speakers.iter().map(|s| (s.id.clone(), key(&s.display_name()))).collect();
        for assignment in assignments {
            let Some(speaker) = speakers.iter().find(|s| s.id == assignment.speaker_id) else { continue };
            let name = key(&assignment.name);
            final_names.insert(speaker.id.clone(), if name.is_empty() { key(&speaker.default_name()) } else { name });
        }
        // Speakers named in the dialog first, so they keep their passages' label.
        let named: Vec<&SpeakerRow> = assignments
            .iter()
            .filter_map(|a| speakers.iter().find(|s| s.id == a.speaker_id))
            .collect();
        let named_ids: HashSet<&str> = named.iter().map(|s| s.id.as_str()).collect();
        let ordered: Vec<&SpeakerRow> =
            named.iter().copied().chain(speakers.iter().filter(|s| !named_ids.contains(s.id.as_str()))).collect();
        let mut survivors: HashMap<String, String> = HashMap::new();
        let mut merged: HashSet<String> = HashSet::new();
        for speaker in ordered {
            let Some(name) = final_names.get(&speaker.id) else { continue };
            if let Some(target) = survivors.get(name) {
                self.merge(&speaker.id, target)?;
                merged.insert(speaker.id.clone());
            } else {
                survivors.insert(name.clone(), speaker.id.clone());
            }
        }
        for assignment in assignments.iter().filter(|a| !merged.contains(&a.speaker_id)) {
            if let Some(speaker) = self.library.speaker(&assignment.speaker_id)? {
                self.set_name(&speaker, assignment.name.trim(), assignment.remember)?;
            }
        }
        self.library.touch_transcript(transcript_id)
    }

    /// Sets the name without looking at other speakers. An empty or default
    /// name restores the default and forgets the voice link.
    fn set_name(&mut self, speaker: &SpeakerRow, name: &str, remember: bool) -> Result<()> {
        let is_default = name.is_empty() || name == speaker.default_name();
        self.library.set_custom_name(&speaker.id, if is_default { None } else { Some(name) })?;
        if is_default || !remember {
            self.voices.unlink(&speaker.id)
        } else {
            self.voices.link(&speaker.id, name, self.recognizes)
        }
    }

    /// Takes the suggested person ("Max?") as the speaker's name.
    pub fn confirm_suggestion(&mut self, speaker_id: &str) -> Result<()> {
        let status = self.voices.statuses(&[speaker_id.to_string()])?.remove(speaker_id);
        if let Some(name) = status.and_then(|s| s.person_name) {
            self.rename(speaker_id, &name, true)?;
        }
        Ok(())
    }

    /// Moves all of the speaker's segments to `target_id` (same transcript)
    /// and removes the speaker: one person the diarization split into
    /// several voices.
    pub fn merge(&mut self, speaker_id: &str, target_id: &str) -> Result<()> {
        let (Some(speaker), Some(target)) = (self.library.speaker(speaker_id)?, self.library.speaker(target_id)?) else {
            return Ok(());
        };
        if speaker.transcript_id != target.transcript_id || speaker.id == target.id {
            return Ok(());
        }
        self.voices.merge(&speaker.id, &target.id, self.recognizes)?;
        self.library.merge_speaker(&speaker.id, &target.id)?;
        self.library.touch_transcript(&speaker.transcript_id)
    }

    /// Moves one passage to another speaker of the same transcript: the
    /// diarization put two people into one voice. The source speaker's voice
    /// is then a mix and no longer teaches anyone's profile. A speaker left
    /// without passages is removed.
    pub fn move_segment(&mut self, segment_id: &str, target_id: &str) -> Result<()> {
        let Some((transcript_id, Some(source_id))) = self.library.segment_owner(segment_id)? else { return Ok(()) };
        let Some(target) = self.library.speaker(target_id)? else { return Ok(()) };
        if source_id == target.id || target.transcript_id != transcript_id {
            return Ok(());
        }
        let source_keeps_passages = self.library.segments_of(&source_id)?.iter().any(|id| id != segment_id);
        self.voices.discard_voice(&source_id)?;
        self.library.set_segment_speaker(segment_id, &target.id)?;
        if !source_keeps_passages {
            self.library.delete_speaker(&source_id)?;
        }
        self.library.touch_transcript(&transcript_id)
    }

    /// Moves one passage to a new numbered speaker ("Sprecher 4"), named
    /// `name` if given (a known person not yet in this transcript). If a
    /// speaker already has that name, the passage goes there.
    pub fn move_to_new_speaker(&mut self, segment_id: &str, name: Option<&str>) -> Result<()> {
        let Some((transcript_id, _)) = self.library.segment_owner(segment_id)? else { return Ok(()) };
        let speakers = self.library.speakers(&transcript_id)?;
        if let Some(name) = name {
            if let Some(existing) = speakers.iter().find(|s| key(&s.display_name()) == key(name)) {
                return self.move_segment(segment_id, &existing.id);
            }
        }
        let number = speakers.iter().filter(|s| s.role == SpeakerRole::Diarized).map(|s| s.number).max().unwrap_or(0) + 1;
        let id = self.library.add_speaker(&transcript_id, SpeakerRole::Diarized, number, name)?;
        self.move_segment(segment_id, &id)
    }

    /// Renames a known person and every speaker named after them.
    pub fn rename_person(&mut self, person_id: &str, name: &str) -> Result<()> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        for label in self.voices.rename(person_id, trimmed)? {
            if self.library.speaker(&label)?.is_some() {
                self.library.set_custom_name(&label, Some(trimmed))?;
            }
        }
        Ok(())
    }
}

/// The name was set from a guess and not confirmed yet.
pub fn is_guess(speaker_custom_name: Option<&str>, state: MatchState, person_name: Option<&str>) -> bool {
    state == MatchState::Suggested && person_name.is_some() && speaker_custom_name == person_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::{draft, new_recording};
    use crate::model::RecordingSource;
    use crate::transcript::SpeakerKey;
    use crate::voices::tests::SyntheticVoices;
    use crate::voices::LabelVoice;

    struct Fixture {
        library: Library,
        voices: VoiceStore,
        transcript: String,
        one: String,
        two: String,
    }

    impl Fixture {
        /// A transcript with two diarized speakers whose voices are known.
        fn new() -> Self {
            let mut library = Library::open_in_memory().unwrap();
            let mut voices = VoiceStore::open_in_memory().unwrap();
            let recording = new_recording(&mut library, "Meeting", RecordingSource::Meeting);
            let (keys, _) = library
                .store_transcript(
                    &recording,
                    &draft(&[("Satz eins.", Some(SpeakerKey::diarized(1))), ("Satz zwei.", Some(SpeakerKey::diarized(2)))]),
                    Some("de"),
                    "test",
                )
                .unwrap();
            let one = keys[&SpeakerKey::diarized(1)].clone();
            let two = keys[&SpeakerKey::diarized(2)].clone();
            let mut synthetic = SyntheticVoices::new();
            let person = synthetic.person();
            voices
                .recognize(
                    &[
                        LabelVoice { label_id: one.clone(), embedding: synthetic.recording(&person, 0.3), speech_seconds: 30.0 },
                        LabelVoice { label_id: two.clone(), embedding: synthetic.recording(&person, 0.3), speech_seconds: 20.0 },
                    ],
                    &recording,
                    true,
                )
                .unwrap();
            let transcript = library.transcript_id(&recording).unwrap().unwrap();
            Self { library, voices, transcript, one, two }
        }

        fn speakers(&mut self) -> Speakers<'_> {
            Speakers { library: &mut self.library, voices: &mut self.voices, recognizes: true }
        }

        fn name(&self, id: &str) -> String {
            self.library.speaker(id).unwrap().unwrap().display_name()
        }

        fn speaker_ids(&self) -> Vec<String> {
            self.library.speakers(&self.transcript).unwrap().into_iter().map(|s| s.id).collect()
        }

        fn add_segment(&mut self, start: f64, text: &str, speaker: &str) -> String {
            let id = uuid::Uuid::new_v4().to_string();
            self.library
                .conn_for_tests()
                .execute(
                    "INSERT INTO segments (id, transcript_id, idx, start, end, text, speaker_id) VALUES (?1, ?2, 9, ?3, ?4, ?5, ?6)",
                    rusqlite::params![id, self.transcript, start, start + 2.0, text, speaker],
                )
                .unwrap();
            id
        }

        fn speaker_of(&self, segment: &str) -> Option<String> {
            self.library.segment_owner(segment).unwrap().unwrap().1
        }
    }

    #[test]
    fn naming_creates_the_person_and_merging_teaches_the_second_voice() {
        let mut f = Fixture::new();
        let (one, two) = (f.one.clone(), f.two.clone());
        f.speakers().rename(&one, "Max", true).unwrap();
        assert_eq!(f.name(&one), "Max");
        assert_eq!(f.voices.people().unwrap()[0].confirmed_samples, 1);

        f.speakers().merge(&two, &one).unwrap();
        assert_eq!(f.speaker_ids(), [one.clone()]);
        assert_eq!(f.voices.people().unwrap()[0].confirmed_samples, 2);
        assert!(f.voices.statuses(&[two]).unwrap().is_empty());
    }

    #[test]
    fn renaming_to_another_speakers_name_merges() {
        let mut f = Fixture::new();
        let (one, two) = (f.one.clone(), f.two.clone());
        f.speakers().rename(&two, "sprecher 1", true).unwrap();
        assert_eq!(f.speaker_ids(), [one]);
    }

    #[test]
    fn moving_passages_separates_two_people_in_one_voice() {
        let mut f = Fixture::new();
        let (one, two) = (f.one.clone(), f.two.clone());
        f.speakers().rename(&one, "Max", true).unwrap();
        assert_eq!(f.voices.people().unwrap()[0].confirmed_samples, 1);

        // "Max" also contains a passage of someone else.
        let extra = f.add_segment(30.0, "Das war Anna.", &one);
        f.speakers().move_to_new_speaker(&extra, None).unwrap();
        let anna = f.speaker_of(&extra).unwrap();
        assert_ne!(anna, one);
        assert_eq!(f.library.speaker(&anna).unwrap().unwrap().number, 3);
        assert_eq!(f.name(&one), "Max");
        // The mixed voice no longer teaches Max's profile.
        assert_eq!(f.voices.people().unwrap()[0].confirmed_samples, 0);
        assert!(f.voices.statuses(&[one.clone()]).unwrap().is_empty());

        // A known person who isn't a speaker here yet becomes one; the next
        // passage for them joins that speaker.
        let first = f.add_segment(40.0, "Hallo.", &one);
        let second = f.add_segment(43.0, "Tschüss.", &one);
        f.speakers().move_to_new_speaker(&first, Some("Lena")).unwrap();
        f.speakers().move_to_new_speaker(&second, Some("lena")).unwrap();
        let lena = f.speaker_of(&first).unwrap();
        assert_eq!(f.name(&lena), "Lena");
        assert_eq!(f.speaker_of(&second).unwrap(), lena);

        // Moving the last passage of a speaker removes it.
        let only = f.library.segments_of(&two).unwrap()[0].clone();
        f.speakers().move_segment(&only, &anna).unwrap();
        assert!(!f.speaker_ids().contains(&two));
        assert_eq!(f.speaker_ids().len(), 3);
    }

    #[test]
    fn swapping_two_guessed_names_keeps_both_speakers() {
        let mut f = Fixture::new();
        let (one, two, transcript) = (f.one.clone(), f.two.clone(), f.transcript.clone());
        f.library.set_custom_name(&one, Some("Max")).unwrap();
        f.library.set_custom_name(&two, Some("Maximilian")).unwrap();
        f.speakers()
            .assign_names(
                &transcript,
                &[
                    NameAssignment { speaker_id: one.clone(), name: "Maximilian".into(), remember: true },
                    NameAssignment { speaker_id: two.clone(), name: "Max".into(), remember: true },
                ],
            )
            .unwrap();
        assert_eq!(f.speaker_ids().len(), 2);
        assert_eq!(f.name(&one), "Maximilian");
        assert_eq!(f.name(&two), "Max");

        // The same final name for both still merges them; the renamed speaker stays.
        f.speakers()
            .assign_names(&transcript, &[NameAssignment { speaker_id: two.clone(), name: "maximilian".into(), remember: true }])
            .unwrap();
        assert_eq!(f.speaker_ids(), [two]);
    }

    #[test]
    fn guests_are_named_without_being_remembered() {
        let mut f = Fixture::new();
        let one = f.one.clone();
        f.speakers().rename(&one, "Besuch", false).unwrap();
        assert_eq!(f.name(&one), "Besuch");
        assert!(f.voices.people().unwrap().is_empty());
    }

    #[test]
    fn renaming_a_person_renames_their_speakers() {
        let mut f = Fixture::new();
        let one = f.one.clone();
        f.speakers().rename(&one, "Max", true).unwrap();
        let person = f.voices.people().unwrap()[0].id.clone();
        f.speakers().rename_person(&person, "Max K.").unwrap();
        assert_eq!(f.name(&one), "Max K.");
        assert_eq!(f.voices.people().unwrap()[0].name, "Max K.");
        assert_eq!(f.speaker_ids().len(), 2);
    }

    #[test]
    fn an_empty_name_restores_the_default() {
        let mut f = Fixture::new();
        let one = f.one.clone();
        f.speakers().rename(&one, "Max", true).unwrap();
        f.speakers().rename(&one, "  ", true).unwrap();
        assert_eq!(f.name(&one), "Sprecher 1");
        assert_eq!(f.voices.people().unwrap()[0].samples, 0);
    }
}
