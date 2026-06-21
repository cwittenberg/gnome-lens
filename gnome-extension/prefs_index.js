// gnome-extension/prefs_index.js
import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export function buildIndexPage(settings, window) {
    const page = new Adw.PreferencesPage({ 
        title: 'Indexation', 
        icon_name: 'folder-saved-search-symbolic' 
    });

    // ==========================================
    // 1. SCOPE GROUP (Full System Toggle)
    // ==========================================
    const scopeGroup = new Adw.PreferencesGroup({ title: 'Indexing Scope' });
    const fullSysRow = new Adw.SwitchRow({
        title: 'Full Home Directory Indexation',
        subtitle: 'Index all files recursively inside your home folder. Warning: Can be resource intensive.'
    });
    settings.bind('index-full-system', fullSysRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    scopeGroup.add(fullSysRow);
    page.add(scopeGroup);

    // ==========================================
    // 2. TARGET PATHS GROUP
    // ==========================================
    const pathGroup = new Adw.PreferencesGroup({ 
        title: 'Specific Target Directories',
        description: 'Directories to recursively index when Full Home Indexation is disabled.'
    });
    
    let pathRows = [];
    const updatePaths = () => {
        pathRows.forEach(row => pathGroup.remove(row));
        pathRows = [];
        
        let paths = settings.get_strv('index-paths') || [];
        for (let p of paths) {
            let row = new Adw.ActionRow({ title: p });
            let delBtn = new Gtk.Button({
                icon_name: 'user-trash-symbolic',
                valign: Gtk.Align.CENTER,
                margin_end: 8
            });
            delBtn.add_css_class('destructive-action');
            delBtn.connect('clicked', () => {
                let newPaths = settings.get_strv('index-paths').filter(x => x !== p);
                settings.set_strv('index-paths', newPaths);
                updatePaths();
            });
            row.add_suffix(delBtn);
            pathGroup.add(row);
            pathRows.push(row);
        }
    };

    const addPathRow = new Adw.ActionRow({ title: 'Add Directory...' });
    const addPathBtn = new Gtk.Button({
        icon_name: 'list-add-symbolic',
        valign: Gtk.Align.CENTER,
        margin_end: 8
    });
    addPathBtn.add_css_class('suggested-action');
    addPathBtn.connect('clicked', () => {
        let dialog = new Gtk.FileDialog({ title: 'Select Directory to Index' });
        dialog.select_folder(window, null, (dlg, res) => {
            try {
                let file = dlg.select_folder_finish(res);
                if (file) {
                    let path = file.get_path();
                    let home = GLib.get_home_dir();
                    // Elegantly shorten home paths to ~/
                    if (path.startsWith(home)) {
                        path = '~' + path.substring(home.length);
                    }
                    
                    let currentPaths = settings.get_strv('index-paths') || [];
                    if (!currentPaths.includes(path)) {
                        currentPaths.push(path);
                        settings.set_strv('index-paths', currentPaths);
                        updatePaths();
                    }
                }
            } catch (e) {
                // User cancelled dialog
            }
        });
    });
    addPathRow.add_suffix(addPathBtn);
    pathGroup.add(addPathRow);
    updatePaths();

    // Disable the target paths UI if the user turns on Full System indexation
    settings.connect('changed::index-full-system', () => {
        pathGroup.set_sensitive(!settings.get_boolean('index-full-system'));
    });
    pathGroup.set_sensitive(!settings.get_boolean('index-full-system'));
    
    page.add(pathGroup);

    // ==========================================
    // 3. BLACKLIST GROUP
    // ==========================================
    const blacklistGroup = new Adw.PreferencesGroup({ 
        title: 'Blacklisted Names',
        description: 'Folder or file names that will be explicitly ignored during indexing (e.g. node_modules, .git).'
    });

    let blacklistRows = [];
    const updateBlacklist = () => {
        blacklistRows.forEach(row => blacklistGroup.remove(row));
        blacklistRows = [];
        
        let items = settings.get_strv('index-blacklist') || [];
        for (let item of items) {
            let row = new Adw.ActionRow({ title: item });
            let delBtn = new Gtk.Button({
                icon_name: 'user-trash-symbolic',
                valign: Gtk.Align.CENTER,
                margin_end: 8
            });
            delBtn.add_css_class('destructive-action');
            delBtn.connect('clicked', () => {
                let newItems = settings.get_strv('index-blacklist').filter(x => x !== item);
                settings.set_strv('index-blacklist', newItems);
                updateBlacklist();
            });
            row.add_suffix(delBtn);
            blacklistGroup.add(row);
            blacklistRows.push(row);
        }
    };

    const addBlacklistRow = new Adw.EntryRow({ 
        title: 'Add new ignore rule...',
        show_apply_button: true 
    });
    addBlacklistRow.connect('apply', () => {
        let text = addBlacklistRow.get_text().trim();
        if (text) {
            let items = settings.get_strv('index-blacklist') || [];
            if (!items.includes(text)) {
                items.push(text);
                settings.set_strv('index-blacklist', items);
                updateBlacklist();
            }
            addBlacklistRow.set_text('');
        }
    });
    blacklistGroup.add(addBlacklistRow);
    updateBlacklist();
    
    page.add(blacklistGroup);

    return page;
}