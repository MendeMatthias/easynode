// The app's own version, from package.json, so the preview's header never
// shows an old one after a release bump.
import pkg from "../../package.json";

export const getVersion = async (): Promise<string> => pkg.version;
