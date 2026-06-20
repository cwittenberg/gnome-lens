import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import { ExtensionPreferences } from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

export default class GnomeLensPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings('org.gnome.shell.extensions.gnome-lens');
        const page = new Adw.PreferencesPage();
        
        const uiGroup = new Adw.PreferencesGroup({ title: 'User Interface' });
        
        const animationRow = new Adw.SwitchRow({
            title: 'Show LLM Animations',
            subtitle: 'Display bouncing dots while synthesizing data',
        });
        settings.bind('show-llm-animations', animationRow, 'active', Gio.SettingsBindFlags.DEFAULT);
        uiGroup.add(animationRow);
        
        page.add(uiGroup);

        const historyGroup = new Adw.PreferencesGroup({ title: 'Search History' });

        const historySwitch = new Adw.SwitchRow({
            title: 'Enable History',
            subtitle: 'Save recent searches for quick access in the tray context menu',
        });
        settings.bind('enable-history', historySwitch, 'active', Gio.SettingsBindFlags.DEFAULT);
        historyGroup.add(historySwitch);

        const clearBtn = new Gtk.Button({
            label: 'Clear History',
            valign: Gtk.Align.CENTER,
        });
        clearBtn.connect('clicked', () => {
            settings.set_strv('search-history', []);
        });

        const clearRow = new Adw.ActionRow({
            title: 'Clear Saved History',
        });
        clearRow.add_suffix(clearBtn);
        historyGroup.add(clearRow);

        page.add(historyGroup);
        window.add(page);
    }
}