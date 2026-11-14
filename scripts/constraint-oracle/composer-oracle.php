<?php
// Asks Composer's own semver library (inside composer.phar) which of a list of versions satisfy each constraint.
// It is the reference the Rust constraint engine is compared with (crates/luna-core/tests/fixtures/constraints/).
//
//   php composer-oracle.php <composer.phar> <cases.json>      (prints JSON)
//
// cases.json: { "constraints": ["^8.1", ...], "versions": ["8.1.0", ...] }
// Output: [{ "constraint", "valid", "error"? , "normalized"?, "matches"? }, ...]
// Run by generate.mjs, which also checks composer.phar against the checksum in registry/registry.json.

if ($argc < 3) {
    fwrite(STDERR, "usage: php composer-oracle.php <composer.phar> <cases.json>\n");
    exit(2);
}
require 'phar://' . $argv[1] . '/vendor/autoload.php';

use Composer\Semver\Semver;
use Composer\Semver\VersionParser;

$cases = json_decode(file_get_contents($argv[2]), true);
$parser = new VersionParser();
$out = [];
foreach ($cases['constraints'] as $constraint) {
    try {
        $parsed = $parser->parseConstraints($constraint);
    } catch (\Throwable $e) {
        $out[] = ['constraint' => $constraint, 'valid' => false, 'error' => $e->getMessage()];
        continue;
    }
    $matches = [];
    foreach ($cases['versions'] as $version) {
        if (Semver::satisfies($version, $constraint)) {
            $matches[] = $version;
        }
    }
    $out[] = ['constraint' => $constraint, 'valid' => true, 'normalized' => (string) $parsed, 'matches' => $matches];
}
echo json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES), "\n";
