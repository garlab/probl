"""Audit current semantics and executable equivalents of proposed migrations.

Does not implement the proposed operator or static rejection rules.
"""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / 'target/release/probl'
WORK = None

REWRITES = {
    '01_tour': [
        ('2d6 >= 10 as', '2d6 >= one_of([10]) as'),
        ('d20 + 5 >= 15 as', 'd20 + one_of([5]) >= one_of([15]) as'),
        ('while d6 != 6 {', 'while P(d6 != one_of([6])) {'),
    ],
    '03_rpg_duel': [
        ('let dmg ~ a.dice + a.dice + a.bonus',
         'let base ~ a.dice + a.dice\n    let dmg = base + a.bonus'),
        ('let dmg ~ a.dice + a.bonus',
         'let base ~ a.dice\n    let dmg = base + a.bonus'),
    ],
    '05_snakes_and_ladders': [
        ('report game <= 20 as', 'report game <= one_of([20]) as'),
        ('report game >= mine as', 'report game >= one_of([mine]) as'),
    ],
    '09_roadmap': [
        ('~ if 35% { 1 to 3 } else { 0 }', '= if 35% { let delay ~ 1 to 3; delay } else { 0 }'),
        ('~ if 25% { 1 to 4 } else { 0 }', '= if 25% { let delay ~ 1 to 4; delay } else { 0 }'),
        ('~ if 30% { 1 to 3 } else { 0 }', '= if 30% { let delay ~ 1 to 3; delay } else { 0 }'),
    ],
    '10_quantum_key': [
        ('binomial(compared, error) > 0 by', 'binomial(compared, error) > one_of([0]) by'),
    ],
    '18_correlated_losses': [
        ('100 * binomial(3, site_risk)', 'one_of([100]) * binomial(3, site_risk)'),
        ('shared_loss >= 200 as', 'shared_loss >= one_of([200]) as'),
        ('independent_loss >= 200 as', 'independent_loss >= one_of([200]) as'),
        ('mean(max(shared_loss - 100, 0))',
         'mean(simulate { let loss ~ shared_loss; max(loss - 100, 0) })'),
        ('mean(max(independent_loss - 100, 0))',
         'mean(simulate { let loss ~ independent_loss; max(loss - 100, 0) })'),
    ],
}

PROBES = {
    'independent_distribution_algebra': 'let d = d6\nreport d - d as "difference"\nreport d == d as "same"',
    'shared_draw_identity': 'let x ~ d6\nlet y = x\nreport x - y as "difference"\nreport x == y as "same"',
    'point_distribution_shift': 'report d6 + one_of([3]) as "shifted"',
    'original_observation_trap': 'let x = 3d8\nobserve x > 10\nreport x',
    'conditional_draw': 'let x ~ 3d8\nobserve x > 10\nreport x',
    'implicit_boolean_condition': 'let e = d6 > d6\nobserve e\nreport e',
    'scalar_queries': 'report mean(3)\nreport sd(3)\nreport support(3)',
    'list_of_distributions': 'let xs = [d6, d6]\nreport sum(xs)',
    'distribution_of_lists': 'let xs = roll(4, d6)\nreport xs.highest(3).sum()',
    'scalar_distribution_map': 'report d6.map(x -> x + 1)',
    'projected_marginals': 'let pair = simulate { let x ~ d2; { a: x, b: x } }\nreport pair.a == pair.b',
    'joint_record_transform': 'let pair = simulate { let x ~ d2; { a: x, b: x } }\nreport simulate { let p ~ pair; p.a == p.b }',
    'constructor_flattens_parameter': 'let p = one_of([10%, 90%])\nlet d = bernoulli(p)\nreport simulate { let a ~ d; let b ~ d; a and b }',
    'shared_parameter_draw': 'let p ~ one_of([10%, 90%])\nlet d = bernoulli(p)\nreport simulate { let a ~ d; let b ~ d; a and b }',
    'constructor_mixes_distributions': 'report one_of([d2, 10])',
    'draw_scalar_passthrough': 'let x ~ 3\nreport x',
    'draw_does_not_descend_into_list': 'let xs ~ [d2, d2]\nreport xs',
    'continuous_distribution_addition': 'report normal(0, 1) + normal(0, 1)',
    'continuous_query': 'report 1 - cdf(normal(0, 1), 1.96)',
    'finite_transform': 'let d = d6\nreport mean(simulate { let x ~ d; max(x - 3, 0) })',
    'continuous_transform_limit': '@mode sample(runs: 10, seed: 1)\nreport mean(simulate { let x ~ normal(0, 1); max(x, 0) })',
    'callback_draw_restriction': 'report [1, 2].map(x -> { let y ~ d6; x + y })',
    'bag_take': 'var deck = bag([1: 1, 2: 1])\nlet a = deck.take()\nlet b = deck.take()\nreport a != b',
    'annotated_invalid_binding': 'let x: int = d6\nreport x',
    'unreachable_type_error': 'if false { let x: int = d6 }\nreport true',
}

PARTIAL = '@epsilon 0.1\nlet e = simulate { var n = 0; while 50% { n += 1 }; n == 0 }\n'
PROBES.update({
    'partial_condition': PARTIAL + 'report if e { true } else { false } as "event"',
    'partial_draw': PARTIAL + 'let b ~ e\nreport if b { true } else { false } as "event"',
    'partial_probability': PARTIAL + 'report if P(e) { true } else { false } as "event"',
    'partial_observation': PARTIAL + 'observe e\nreport true',
    'partial_observation_probability': PARTIAL + 'observe P(e)\nreport true',
})

# These checks characterize the current engine, including existing limitations.
# They do not claim the proposed static restrictions have been implemented.
RUNTIME_ERRORS = {
    'scalar_distribution_map', 'continuous_distribution_addition',
    'continuous_transform_limit', 'callback_draw_restriction',
    'annotated_invalid_binding',
}
EXPECTED_TEXT = {
    'independent_distribution_algebra': '16.67%',
    'shared_draw_identity': '100.00%',
    'original_observation_trap': 'mean 13.50',
    'conditional_draw': 'mean 15.11',
    'implicit_boolean_condition': 'e    41.67%',
    'scalar_queries': 'support(3)    [3]',
    'list_of_distributions': 'mean 7.00',
    'distribution_of_lists': 'mean 12.24',
    'projected_marginals': '50.00%',
    'joint_record_transform': '100.00%',
    'constructor_flattens_parameter': '25.00%',
    'shared_parameter_draw': '41.00%',
    'draw_does_not_descend_into_list': '[dist(',
    'bag_take': '100.00%',
    'partial_condition': '50.00%–56.25%',
    'partial_draw': '50.00%–56.25%',
    'partial_probability': 'event    50.00%\n',
    'partial_observation': 'evidence 50.00%–56.25%',
    'partial_observation_probability': 'evidence 50.00%\n',
}


def invoke(path, command='run', mode=None):
    args = [str(CLI), command, str(path)]
    if command == 'run':
        args += ['--today', '2026-09-29', '--threads', '2']
        if mode == 'sample':
            args += ['--runs', '1000', '--seed', '7']
    p = subprocess.run(args, cwd=ROOT, capture_output=True, text=True, timeout=120)
    return {'code': p.returncode, 'stdout': p.stdout, 'stderr': p.stderr}


def example(path):
    source = path.read_text()
    # Use absolute data paths for scratch files outside examples/.
    source = source.replace('read("data/', 'read("' + str(ROOT / 'examples/data') + '/')
    path_before = WORK / (path.stem + '-before.probl')
    path_before.write_text(source)
    changed = source
    for old, new in REWRITES.get(path.stem, []):
        assert old in changed, (path.stem, old)
        changed = changed.replace(old, new, 1)
    path_after = WORK / (path.stem + '-after.probl')
    path_after.write_text(changed)
    before = invoke(path_before)
    after = invoke(path_after) if changed != source else before
    same = before['code'] == after['code'] == 0 and before['stdout'] == after['stdout']
    result = {'name': path.stem, 'rewritten': changed != source, 'same': same, 'before': before, 'after': after}
    print(f"{'same' if same else 'DIFFERENT'} {path.stem}", flush=True)
    return result


def probe(item):
    name, source = item
    path = WORK / (name + '.probl')
    path.write_text(source + '\n')
    check = invoke(path, 'check')
    run = invoke(path)
    expected_code = 1 if name in RUNTIME_ERRORS else 0
    passed = check['code'] == 0 and run['code'] == expected_code
    if name in EXPECTED_TEXT:
        passed = passed and EXPECTED_TEXT[name] in run['stdout']
    print(f"{'pass' if passed else 'FAIL'} probe {name}: check={check['code']} run={run['code']}", flush=True)
    return {'name': name, 'source': source, 'passed': passed, 'check': check, 'run': run}


if __name__ == '__main__':
    if not CLI.is_file():
        raise SystemExit('Build the CLI first: cargo build --release -p probl-cli')
    WORK = Path(tempfile.mkdtemp(prefix='probl-dist-survey-'))
    print(f'Artifacts: {WORK}', flush=True)
    with ThreadPoolExecutor(max_workers=2) as pool:
        examples = list(pool.map(example, sorted((ROOT / 'examples').glob('*.probl'))))
        probes = list(pool.map(probe, PROBES.items()))
    result = {'directory': str(WORK), 'examples': examples, 'probes': probes}
    (WORK / 'results.json').write_text(json.dumps(result, indent=2))
    assert all(x['same'] for x in examples), 'migration output differences; inspect results.json'
    assert all(x['passed'] for x in probes), 'changed probe behavior; inspect results.json'
    print(f'{len(examples)} example comparisons and {len(probes)} probes passed.', flush=True)
