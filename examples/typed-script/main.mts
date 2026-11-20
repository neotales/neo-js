import { arch, platform } from "neo:os";
import { stdout } from "neo:process"; // Requires --allow-stdio.

const release: string | undefined = os.version;
const suffix = release === undefined ? "" : ` (release ${release})`;
stdout.write(`neojs: ${platform}/${arch}${suffix}\n`);
