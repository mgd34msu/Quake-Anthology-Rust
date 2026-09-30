//! Application audio command names and documentation.
//!
//! Port of donor `src/app/bootstrap/audio/commands.ts`
//! (`applicationAudioCommands`, `cdCommandDocumentation`,
//! `musicCommandDocumentation`). Pure data.

/// Console commands owned by application audio.
pub const APPLICATION_AUDIO_COMMANDS: [&str; 10] = [
    "snd_restart",
    "cd",
    "music",
    "soundinfo",
    "soundlist",
    "play",
    "stopsound",
    "s_info",
    "s_list",
    "s_stop",
];

/// Command documentation: summary, usage, and examples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDocumentation {
    /// One-line summary.
    pub summary: &'static str,
    /// Usage line.
    pub usage: &'static str,
    /// Example invocations.
    pub examples: &'static [&'static str],
    /// Allowed subcommand values, when the command takes an enum.
    pub allowed_values: &'static [&'static str],
}

/// `cd` command documentation.
pub const CD_COMMAND_DOCUMENTATION: CommandDocumentation = CommandDocumentation {
    summary: "Control the selected soundtrack: play or loop a numbered track, stop, pause, resume, enable, reset, remap or inspect it.",
    usage: "cd <play|loop> <track> | cd <stop|pause|resume|on|off|reset|info> | cd remap [tracks...]",
    examples: &[
        "cd loop 2",
        "cd pause",
        "cd resume",
        "cd stop",
        "cd remap 1 3",
        "cd info",
    ],
    allowed_values: &[
        "play", "loop", "stop", "pause", "resume", "on", "off", "reset", "remap", "info",
    ],
};

/// `music` command documentation.
pub const MUSIC_COMMAND_DOCUMENTATION: CommandDocumentation = CommandDocumentation {
    summary: "Play a named soundtrack once, or an intro followed by a looping track, using the current content and music gain.",
    usage: "music <intro> [loop]",
    examples: &["music music/win", "music music/intro music/loop"],
    allowed_values: &[],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_audio_commands() {
        assert_eq!(
            APPLICATION_AUDIO_COMMANDS.as_slice(),
            [
                "snd_restart",
                "cd",
                "music",
                "soundinfo",
                "soundlist",
                "play",
                "stopsound",
                "s_info",
                "s_list",
                "s_stop",
            ]
        );
    }

    #[test]
    fn documents_cd_command() {
        assert!(CD_COMMAND_DOCUMENTATION.summary.contains("numbered track"));
        assert_eq!(
            CD_COMMAND_DOCUMENTATION.usage,
            "cd <play|loop> <track> | cd <stop|pause|resume|on|off|reset|info> | cd remap [tracks...]"
        );
        assert_eq!(CD_COMMAND_DOCUMENTATION.examples.len(), 6);
        assert_eq!(CD_COMMAND_DOCUMENTATION.allowed_values.len(), 10);
        assert!(CD_COMMAND_DOCUMENTATION.allowed_values.contains(&"remap"));
    }

    #[test]
    fn documents_music_command() {
        assert_eq!(MUSIC_COMMAND_DOCUMENTATION.usage, "music <intro> [loop]");
        assert_eq!(MUSIC_COMMAND_DOCUMENTATION.examples.len(), 2);
        assert!(MUSIC_COMMAND_DOCUMENTATION.allowed_values.is_empty());
    }
}
