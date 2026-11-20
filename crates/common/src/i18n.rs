use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static LANG: RwLock<Option<&'static HashMap<&'static str, &'static str>>> = RwLock::new(None);

pub const LANGUAGES: [(&str, &str); 2] = [("en", "English"), ("de", "Deutsch")];

fn german() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| GERMAN.iter().copied().collect())
}

pub fn system_language() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() && v != "C" && v != "POSIX" {
                return v.chars().take(2).collect::<String>().to_lowercase();
            }
        }
    }
    "en".to_string()
}

pub fn set_language(code: &str) {
    let code = if code.is_empty() { system_language() } else { code.to_string() };
    let table = match code.as_str() {
        "de" => Some(german()),
        _ => None,
    };
    if let Ok(mut lang) = LANG.write() {
        *lang = table;
    }
}

static EXTRA_GERMAN: RwLock<Vec<&'static [(&'static str, &'static str)]>> = RwLock::new(Vec::new());

pub fn register_german(table: &'static [(&'static str, &'static str)]) {
    if let Ok(mut extra) = EXTRA_GERMAN.write() {
        extra.push(table);
    }
}

pub fn tr(key: &str) -> String {
    if let Ok(lang) = LANG.read() {
        if let Some(table) = *lang {
            if let Some(v) = table.get(key) {
                return v.to_string();
            }
            if std::ptr::eq(table, german()) {
                if let Ok(extra) = EXTRA_GERMAN.read() {
                    for t in extra.iter() {
                        if let Some((_, v)) = t.iter().find(|(k, _)| *k == key) {
                            return v.to_string();
                        }
                    }
                }
            }
        }
    }
    key.to_string()
}

pub fn trf(key: &str, args: &[&str]) -> String {
    let template = tr(key);
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('s') | Some('d') => {
                    chars.next();
                    if let Some(a) = args.next() {
                        out.push_str(a);
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

pub fn duration_text(total_secs: f64) -> String {
    let total = total_secs.round().max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    let unit = |n: u64, one: &str, many: &str| {
        if n == 1 {
            trf(one, &[&n.to_string()])
        } else {
            trf(many, &[&n.to_string()])
        }
    };
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(unit(h, "%d hour", "%d hours"));
    }
    if m > 0 {
        parts.push(unit(m, "%d minute", "%d minutes"));
    }
    if s > 0 || parts.is_empty() {
        parts.push(unit(s, "%d second", "%d seconds"));
    }
    parts.join(", ")
}

pub fn clock_text(total_secs: f64) -> String {
    let total = total_secs.max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

const GERMAN: &[(&str, &str)] = &[
    ("Press %s to open the overlay", "Drücke %s, um das Overlay zu öffnen"),
    ("Record", "Aufnahme"),
    ("Instant Replay", "Instant Replay"),
    ("Livestream", "Livestream"),
    ("Settings", "Einstellungen"),
    ("Screenshot", "Screenshot"),
    ("Gallery", "Galerie"),
    ("Off", "Aus"),
    ("On", "An"),
    ("Not recording", "Nimmt nicht auf"),
    ("Recording", "Nimmt auf"),
    ("Not streaming", "Streamt nicht"),
    ("Streaming", "Streamt"),
    ("Paused", "Pausiert"),
    ("Start", "Starten"),
    ("Stop", "Stoppen"),
    ("Stop and save", "Stoppen und speichern"),
    ("Pause", "Pausieren"),
    ("Unpause", "Fortsetzen"),
    ("Save", "Speichern"),
    ("Save %s", "%s speichern"),
    ("Save 30 sec", "30 Sek. speichern"),
    ("Save 1 min", "1 Min. speichern"),
    ("Save 5 min", "5 Min. speichern"),
    ("Save 10 min", "10 Min. speichern"),
    ("Turn on", "Einschalten"),
    ("Turn off", "Ausschalten"),
    ("Recording has been paused", "Aufnahme wurde pausiert"),
    ("Recording has been unpaused", "Aufnahme wird fortgesetzt"),
    ("Started recording %s", "Aufnahme von %s gestartet"),
    ("Saved a %s recording of %s", "Aufnahme (%s) von %s gespeichert"),
    ("Failed to start/save recording", "Aufnahme konnte nicht gestartet/gespeichert werden"),
    ("Replay stopped", "Replay gestoppt"),
    ("Started replaying %s", "Replay von %s gestartet"),
    ("Saving replay, this might take some time", "Replay wird gespeichert, das kann etwas dauern"),
    ("Saved a %s replay of %s", "Replay (%s) von %s gespeichert"),
    ("Replay stopped because of an error", "Replay wegen eines Fehlers gestoppt"),
    ("Streaming has stopped", "Stream wurde beendet"),
    ("Started streaming %s", "Stream von %s gestartet"),
    ("Streaming stopped because of an error", "Stream wegen eines Fehlers beendet"),
    ("Saved a screenshot of %s", "Screenshot von %s gespeichert"),
    ("Failed to take a screenshot", "Screenshot fehlgeschlagen"),
    ("Screenshot copied to clipboard", "Screenshot in die Zwischenablage kopiert"),
    (
        "Portal selection reset. The portal will ask what you want to record the next time you use it.",
        "Portal-Auswahl zurückgesetzt. Beim nächsten Mal fragt das Portal erneut, was aufgenommen werden soll.",
    ),
    (
        "Unable to start recording when replay is turned on. Turn off replay before starting recording.",
        "Aufnahme nicht möglich, solange Replay an ist. Schalte Replay zuerst aus.",
    ),
    (
        "Unable to start streaming when replay is turned on. Turn off replay before starting streaming.",
        "Streamen nicht möglich, solange Replay an ist. Schalte Replay zuerst aus.",
    ),
    (
        "Unable to start streaming when recording. Stop recording before starting streaming.",
        "Streamen nicht möglich während einer Aufnahme. Stoppe zuerst die Aufnahme.",
    ),
    (
        "Unable to start recording when streaming. Stop streaming before starting recording.",
        "Aufnahme nicht möglich während eines Streams. Beende zuerst den Stream.",
    ),
    (
        "Unable to start replay when recording. Stop recording before starting replay.",
        "Replay nicht möglich während einer Aufnahme. Stoppe zuerst die Aufnahme.",
    ),
    (
        "Unable to start replay when streaming. Stop streaming before starting replay.",
        "Replay nicht möglich während eines Streams. Beende zuerst den Stream.",
    ),
    (
        "Failed to start streaming, the stream URL is empty. Please set a stream URL in settings",
        "Stream konnte nicht starten, die Stream-URL ist leer. Bitte in den Einstellungen setzen",
    ),
    (
        "Failed to start streaming, the WHIP stream URL should start with http:// or https://",
        "Stream konnte nicht starten, die WHIP-URL muss mit http:// oder https:// beginnen",
    ),
    ("Failed to launch the recorder", "Recorder konnte nicht gestartet werden"),
    ("Microphone muted", "Mikrofon stummgeschaltet"),
    ("Microphone unmuted", "Mikrofon aktiv"),
    ("Chapter marker added", "Kapitelmarke gesetzt"),
    ("Low disk space: %s GB left", "Wenig Speicherplatz: noch %s GB frei"),
    ("this monitor", "diesem Monitor"),
    ("the main monitor", "dem Hauptmonitor"),
    ("window", "Fenster"),
    ("focused", "fokussiertes Fenster"),
    ("region", "Bereich"),
    ("portal", "Portal"),
    ("%d second", "%d Sekunde"),
    ("%d seconds", "%d Sekunden"),
    ("%d minute", "%d Minute"),
    ("%d minutes", "%d Minuten"),
    ("%d hour", "%d Stunde"),
    ("%d hours", "%d Stunden"),
    ("Accent color", "Akzentfarbe"),
    ("Use rounded corners", "Abgerundete Ecken"),
    ("Red", "Rot"),
    ("Green", "Grün"),
    ("Blue", "Blau"),
    ("Start program on system startup?", "Programm beim Systemstart starten?"),
    ("Yes", "Ja"),
    ("No", "Nein"),
    ("Enable keyboard hotkeys?", "Tastatur-Hotkeys aktivieren?"),
    (
        "Yes, but only grab virtual devices (supports some input remapping software)",
        "Ja, aber nur virtuelle Geräte (unterstützt manche Remapping-Software)",
    ),
    (
        "Yes, but don't grab devices (supports all input remapping software)",
        "Ja, aber Geräte nicht exklusiv belegen (unterstützt jede Remapping-Software)",
    ),
    (
        "Press ESC to go back to the previous page/close the UI.",
        "ESC drücken, um zurückzugehen oder das Overlay zu schließen.",
    ),
    ("Clear hotkeys", "Hotkeys leeren"),
    ("Reset hotkeys to default", "Hotkeys zurücksetzen"),
    ("Enable controller hotkeys?", "Controller-Hotkeys aktivieren?"),
    ("Press", "Drücke"),
    ("and", "und"),
    ("Notification speed:", "Benachrichtigungstempo:"),
    ("Normal", "Normal"),
    ("Fast", "Schnell"),
    ("Language:", "Sprache:"),
    ("System language", "Systemsprache"),
    (
        "Exclude personal metadata from the video (such as audio track title)",
        "Persönliche Metadaten ausschließen (z. B. Titel der Tonspur)",
    ),
    ("Exit program", "Programm beenden"),
    ("Global", "Allgemein"),
    ("Back", "Zurück"),
    ("Other settings", "Weitere Einstellungen"),
    ("Appearance", "Darstellung"),
    ("Startup", "Autostart"),
    ("Keyboard hotkeys", "Tastatur-Hotkeys"),
    ("Controller hotkeys", "Controller-Hotkeys"),
    ("Application info", "Programminfo"),
    ("Version: %s", "Version: %s"),
    ("GPU vendor: %s", "GPU-Hersteller: %s"),
    ("Press a key combination to use for the hotkey: \"%s\"", "Drücke eine Tastenkombination für: \"%s\""),
    ("Press Esc to cancel or Backspace to remove the hotkey.", "Esc zum Abbrechen, Rücktaste zum Entfernen."),
    ("The hotkey %s is already used for something else", "Der Hotkey %s wird bereits anderweitig verwendet"),
    ("Show/hide UI", "Overlay ein-/ausblenden"),
    ("Turn replay on/off", "Replay an/aus"),
    ("Save replay", "Replay speichern"),
    ("Save 30 second replay", "30-Sekunden-Replay speichern"),
    ("Save 1 minute replay", "1-Minuten-Replay speichern"),
    ("Save 5 minute replay", "5-Minuten-Replay speichern"),
    ("Save 10 minute replay", "10-Minuten-Replay speichern"),
    ("Start/stop recording", "Aufnahme starten/stoppen"),
    ("Pause/unpause recording", "Aufnahme pausieren/fortsetzen"),
    ("Start/stop recording a region", "Bereich aufnehmen starten/stoppen"),
    ("Start/stop recording a window", "Fenster aufnehmen starten/stoppen"),
    ("Start/stop recording with desktop portal", "Mit Desktop-Portal aufnehmen starten/stoppen"),
    ("Start/stop streaming", "Stream starten/stoppen"),
    ("Take a screenshot", "Screenshot aufnehmen"),
    ("Take a screenshot of a region", "Screenshot eines Bereichs"),
    ("Take a screenshot of a window", "Screenshot eines Fensters"),
    ("Take a screenshot with desktop portal", "Screenshot mit Desktop-Portal"),
    ("Mute/unmute microphone", "Mikrofon stumm/aktiv"),
    ("Add chapter marker", "Kapitelmarke setzen"),
    ("Capture", "Erfassung"),
    ("Image", "Bild"),
    ("File info", "Datei"),
    ("General", "Allgemein"),
    ("Screenshot indicator", "Screenshot-Anzeige"),
    ("Script", "Skript"),
    ("Cancel", "Abbrechen"),
    ("Capture source:", "Aufnahmequelle:"),
    ("Window", "Fenster"),
    ("Region", "Bereich"),
    ("Desktop portal", "Desktop-Portal"),
    ("Monitor %s (%dx%d)", "Monitor %s (%dx%d)"),
    ("Screen", "Bildschirm"),
    ("Image resolution limit:", "Maximale Bildauflösung:"),
    ("Change image resolution", "Bildauflösung ändern"),
    ("Image quality:", "Bildqualität:"),
    ("Medium", "Mittel"),
    ("High", "Hoch"),
    ("Very high (Recommended)", "Sehr hoch (empfohlen)"),
    ("Very high", "Sehr hoch"),
    ("Ultra", "Ultra"),
    ("Record cursor", "Mauszeiger aufnehmen"),
    ("Directory to save screenshots:", "Speicherort für Screenshots:"),
    ("Image format:", "Bildformat:"),
    ("Organize screenshots into folders:", "Screenshots in Ordner sortieren:"),
    ("Name the screenshot file after the captured application:", "Screenshot nach der Anwendung benennen:"),
    ("Yes, but only for screenshots of games", "Ja, aber nur bei Spielen"),
    ("Save screenshot to clipboard", "Screenshot in die Zwischenablage kopieren"),
    ("Save screenshot to disk", "Screenshot auf der Festplatte speichern"),
    ("Show screenshot notifications", "Screenshot-Benachrichtigungen anzeigen"),
    ("Blink scroll lock led when taking a screenshot", "Rollen-LED beim Screenshot blinken lassen"),
    ("Command to open the screenshot with:", "Befehl zum Öffnen des Screenshots:"),
    ("Simple view", "Einfache Ansicht"),
    ("Advanced view", "Erweiterte Ansicht"),
    ("Follow focused window", "Fokussiertem Fenster folgen"),
    ("Focused monitor", "Fokussierter Monitor"),
    ("Main monitor", "Hauptmonitor"),
    ("Area size:", "Bereichsgröße:"),
    ("Video resolution limit:", "Maximale Videoauflösung:"),
    ("Change video resolution", "Videoauflösung ändern"),
    ("Webcam", "Webcam"),
    ("Webcam source:", "Webcam-Quelle:"),
    ("None", "Keine"),
    ("Video format:", "Videoformat:"),
    ("Auto (recommended)", "Automatisch (empfohlen)"),
    ("Flip camera horizontally", "Kamera horizontal spiegeln"),
    ("Audio", "Audio"),
    ("Audio codec:", "Audiocodec:"),
    ("Opus (Recommended)", "Opus (empfohlen)"),
    ("AAC", "AAC"),
    ("Directory to save videos:", "Speicherort für Videos:"),
    ("Output device:", "Ausgabegerät:"),
    ("Input device:", "Eingabegerät:"),
    ("Application:", "Anwendung:"),
    ("Custom...", "Benutzerdefiniert..."),
    ("Organize videos into folders:", "Videos in Ordner sortieren:"),
    ("Name the video file after the captured application:", "Video nach der Anwendung benennen:"),
    ("Yes, but only for videos of games", "Ja, aber nur bei Spielen"),
    ("Add audio track", "Tonspur hinzufügen"),
    ("Add input device", "Eingabegerät hinzufügen"),
    ("Add output device", "Ausgabegerät hinzufügen"),
    ("Add application audio", "Anwendungsaudio hinzufügen"),
    (
        "Record audio from all applications except the selected ones",
        "Alle Anwendungen außer den ausgewählten aufnehmen",
    ),
    ("Video", "Video"),
    ("Video codec:", "Videocodec:"),
    ("Video quality:", "Videoqualität:"),
    ("Video bitrate (Kbps):", "Videobitrate (Kbps):"),
    ("Constant bitrate", "Konstante Bitrate"),
    ("Constant bitrate (Recommended)", "Konstante Bitrate (empfohlen)"),
    ("Frame rate:", "Bildrate:"),
    ("Frame rate mode:", "Bildratenmodus:"),
    ("Auto (Recommended)", "Automatisch (empfohlen)"),
    ("Constant", "Konstant"),
    ("Variable", "Variabel"),
    ("Sync to content", "An Inhalt anpassen"),
    ("Color range:", "Farbbereich:"),
    ("Limited", "Begrenzt"),
    ("Full", "Voll"),
    ("Container:", "Container:"),
    ("Record in low-power mode", "Im Energiesparmodus aufnehmen"),
    ("Enable vulkan video encoding (experimental)", "Vulkan-Videokodierung (experimentell)"),
    ("Show replay notifications", "Replay-Benachrichtigungen anzeigen"),
    ("Show recording notifications", "Aufnahme-Benachrichtigungen anzeigen"),
    ("Show streaming notifications", "Stream-Benachrichtigungen anzeigen"),
    ("Show replay status with scroll lock LED", "Replay-Status über Rollen-LED anzeigen"),
    ("Show recording status with scroll lock LED", "Aufnahmestatus über Rollen-LED anzeigen"),
    ("Show streaming status with scroll lock LED", "Stream-Status über Rollen-LED anzeigen"),
    ("Recording indicator", "Aufnahmeanzeige"),
    ("Audio track #%d", "Tonspur #%d"),
    ("Track title:", "Spurtitel:"),
    (
        "Estimated video file size per minute (excluding audio): %sMB",
        "Geschätzte Dateigröße pro Minute (ohne Audio): %s MB",
    ),
    ("Directory to save replays:", "Speicherort für Replays:"),
    ("Replay indicator", "Replay-Anzeige"),
    ("Autostart", "Autostart"),
    ("Replay duration in seconds:", "Replay-Dauer in Sekunden:"),
    ("Where should temporary replay data be stored?", "Wo sollen die temporären Replay-Daten liegen?"),
    ("RAM", "RAM"),
    ("Disk (Not recommended on SSDs)", "Festplatte (nicht empfohlen bei SSDs)"),
    ("Turn on replay when this program starts", "Replay beim Programmstart einschalten"),
    ("Only turn on replay if a power supply is connected", "Nur mit angeschlossenem Netzteil einschalten"),
    ("Turn on replay when starting a game", "Replay beim Start eines Spiels einschalten"),
    ("Don't turn on replay automatically", "Replay nicht automatisch einschalten"),
    ("Restart replay on save", "Replay nach dem Speichern neu starten"),
    ("Short save duration in seconds:", "Dauer für kurzes Speichern in Sekunden:"),
    ("Estimated max replay size: %sMB", "Geschätzte maximale Replay-Größe: %s MB"),
    ("Stream service:", "Streaming-Dienst:"),
    ("Custom", "Benutzerdefiniert"),
    ("Stream URL:", "Stream-URL:"),
    ("Stream key:", "Stream-Schlüssel:"),
    ("Bearer token (optional):", "Bearer-Token (optional):"),
    ("Streaming info", "Stream-Info"),
    ("Streaming indicator", "Stream-Anzeige"),
    ("Keep a local copy of the stream", "Lokale Kopie des Streams speichern"),
    ("Show the UI", "Overlay anzeigen"),
    ("Quit", "Beenden"),
    ("Show system tray icon", "Tray-Symbol anzeigen"),
    ("Overlay background opacity:", "Deckkraft des Overlay-Hintergrunds:"),
    ("Countdown before recording (seconds):", "Countdown vor der Aufnahme (Sekunden):"),
    ("Warn when free disk space is below (GB):", "Warnen, wenn der freie Speicher unter (GB) fällt:"),
    ("Split recording every (minutes, 0 = off):", "Aufnahme aufteilen alle (Minuten, 0 = aus):"),
    ("Keyframe interval (seconds):", "Keyframe-Intervall (Sekunden):"),
    ("Audio bitrate (Kbps, 0 = auto):", "Audiobitrate (Kbps, 0 = automatisch):"),
    ("Pixel format:", "Pixelformat:"),
    ("No recordings yet", "Noch keine Aufnahmen"),
    ("Open", "Öffnen"),
    ("Open folder", "Ordner öffnen"),
    ("Delete", "Löschen"),
    ("Free space: %s", "Freier Speicher: %s"),
    ("Recordings", "Aufnahmen"),
    ("Replays", "Replays"),
    ("Screenshots", "Screenshots"),
    ("All", "Alle"),
    ("Reset portal selection", "Portal-Auswahl zurücksetzen"),
    ("Starting in %s", "Start in %s"),
];
