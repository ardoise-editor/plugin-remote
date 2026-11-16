//! The texts of the plugin, in English and French.

pub fn t(french: bool, key: &str) -> &'static str {
    let (en, fr) = match key {
        "host" => ("user@host (or user@host:/path/file)", "utilisateur@machine (ou utilisateur@machine:/chemin/fichier)"),
        "folder" => ("Folder", "Dossier"),
        "go" => ("Go", "Aller"),
        "recent" => ("Recent machines", "Machines récentes"),
        "hint" => (
            "Give a machine and a folder: its files open here and go back at each save",
            "Indiquez une machine et un dossier : ses fichiers s'ouvrent ici et repartent à chaque enregistrement",
        ),
        "bad_host" => (
            "Give the machine as user@host (or a name of ~/.ssh/config)",
            "Donnez la machine sous la forme utilisateur@machine (ou un nom de ~/.ssh/config)",
        ),
        "sending" => ("Sending to", "Envoi à"),
        "sent" => ("Sent to", "Envoyé à"),
        "send_failed" => (
            "Not sent to the server (the local copy is saved)",
            "Pas envoyé au serveur (la copie locale est enregistrée)",
        ),
        _ => ("", ""),
    };
    if french {
        fr
    } else {
        en
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_text_exists_in_both_languages() {
        for key in [
            "host",
            "folder",
            "go",
            "recent",
            "hint",
            "bad_host",
            "sending",
            "sent",
            "send_failed",
        ] {
            assert!(
                !super::t(false, key).is_empty() && !super::t(true, key).is_empty(),
                "{key}"
            );
        }
    }
}
