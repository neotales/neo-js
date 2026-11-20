//! Integration tests for the systemd and Windows scheduler modules.
//!
//! The modules are pure text generation: these tests register the optional
//! `neojs-schedulers` extension, run it through the public `NeoJs` API with no
//! permissions, assert exact generated text, and exercise the documented
//! rejection paths. Where the host provides `systemd-analyze` or `xmllint`,
//! the tests also verify the generated files without touching any installed
//! unit or task.

use std::path::Path;
use std::process::Command;

use neojs_core::{NeoJs, Permissions, RuntimeOptions};
use neojs_schedulers::with_schedulers;

fn runtime() -> NeoJs {
    let options = with_schedulers(RuntimeOptions::new()).unwrap();
    NeoJs::new_with_options(Permissions::default(), options).unwrap()
}

fn eval(js: &NeoJs, expression: &str) -> String {
    js.eval(expression).unwrap()
}

fn run(js: &NeoJs, name: &str, source: &str) {
    js.run_module(name, source).unwrap();
}

#[test]
fn deprecated_scheduler_keeps_its_output_and_error_contract() {
    run(
        &runtime(),
        "legacy-scheduler.js",
        r#"
        import * as legacy from 'neo:scheduler';
        import * as systemd from 'neo:os/schedulers/systemd';
        import * as windows from 'neo:os/schedulers/windows';
        if (legacy.SchedulerError === systemd.SchedulerError
            || legacy.SchedulerError === windows.SchedulerError
            || systemd.SchedulerError === windows.SchedulerError) {
          throw Error('scheduler error constructors should be distinct across module specifiers');
        }
        const schedule = { kind: 'weekly', at: '11:15', weekdays: ['wed', 'mon'] };
        const service = { name: 'backup', program: '/bin/true', args: ['a b', '%x'] };
        const timer = { name: service.name, schedule, persistent: true };
        const task = { name: 'Backup', program: 'C:\\backup.exe', startDate: '2026-11-18', schedule };
        const legacyUnits = legacy.systemdUnits({ ...service, ...timer });
        const units = systemd.systemdUnits({ ...service, ...timer });
        if (legacyUnits.service !== units.service || legacyUnits.timer !== units.timer
            || legacy.systemdService(service) !== systemd.systemdService(service)
            || legacy.systemdTimer(timer) !== systemd.systemdTimer(timer)
            || legacy.windowsTask(task) !== windows.windowsTask(task)
            || legacy.MAX_SYSTEMD_LINE_BYTES !== systemd.MAX_SYSTEMD_LINE_BYTES
            || legacy.MAX_WINDOWS_COMMAND_LINE !== windows.MAX_WINDOWS_COMMAND_LINE
            || legacy.WINDOWS_XML_ENCODING !== windows.WINDOWS_XML_ENCODING) {
          throw Error('legacy scheduler output differs');
        }

        function validateError(module, generate) {
          let error;
          try { generate(); } catch (err) { error = err; }
          if (!(error instanceof module.SchedulerError) || error.name !== 'SchedulerError'
              || error.code !== 'SCHEDULER_VALUE' || error.field !== 'program') {
            throw Error('wrong scheduler error: ' + error);
          }
        }
        validateError(legacy, () => legacy.systemdService({ ...service, program: 'relative' }));
        validateError(legacy, () => legacy.windowsTask({ ...task, program: 'relative' }));
        validateError(systemd, () => systemd.systemdService({ ...service, program: 'relative' }));
        validateError(windows, () => windows.windowsTask({ ...task, program: 'relative' }));
        "#,
    );
}

#[test]
fn static_imports_reject_the_other_targets_exports() {
    let js = runtime();
    for (index, (name, export)) in [
        ("neo:os/schedulers/systemd", "windowsTask"),
        ("neo:os/schedulers/windows", "systemdUnits"),
    ]
    .into_iter()
    .enumerate()
    {
        let error = js
            .run_module(
                &format!("foreign-export-{index}.js"),
                &format!("import {{ {export} }} from '{name}';"),
            )
            .unwrap_err();
        assert!(error.contains(export), "{error}");
    }
}

#[test]
fn systemd_daily_units_are_exact_and_need_no_permission() {
    let js = runtime();
    assert_eq!(
        js.eval("typeof fs + ',' + typeof env + ',' + typeof __neoExec")
            .unwrap(),
        "undefined,undefined,undefined"
    );
    run(
        &js,
        "daily.js",
        r#"
        import { systemdUnits } from 'neo:os/schedulers/systemd';
        const units = systemdUnits({
          name: 'backup',
          program: '/bin/true',
          schedule: { kind: 'daily', at: '09:30' },
        });
        globalThis.service = units.service;
        globalThis.timer = units.timer;
        "#,
    );
    assert_eq!(
        eval(&js, "globalThis.service"),
        "[Service]\nType=oneshot\nExecStart=\"/bin/true\"\n"
    );
    assert_eq!(
        eval(&js, "globalThis.timer"),
        "[Timer]\nOnCalendar=*-*-* 09:30:00\nUnit=backup.service\n\n[Install]\nWantedBy=timers.target\n"
    );
}

#[test]
fn systemd_weekly_unit_escapes_arguments_paths_and_whitespace() {
    let js = runtime();
    run(
        &js,
        "weekly.js",
        r#"
        import { systemdUnits } from 'neo:os/schedulers/systemd';
        const units = systemdUnits({
          name: 'nightly-backup',
          description: 'Nightly backup (01)',
          program: '/opt/My App/backup tool',
          args: ['--full', 'a b', '100%', '$HOME', 'quote"x', 'back\\slash', ''],
          workingDirectory: '/var/lib/My App',
          user: 'backup',
          schedule: { kind: 'weekly', at: '02:05', weekdays: ['wed', 'mon'] },
          persistent: true,
        });
        globalThis.service = units.service;
        globalThis.timer = units.timer;
        "#,
    );
    assert_eq!(
        eval(&js, "globalThis.service"),
        r#"[Unit]
Description=Nightly backup (01)

[Service]
Type=oneshot
ExecStart="/opt/My App/backup tool" "--full" "a b" "100%%" "$$HOME" "quote\"x" "back\\slash" ""
User=backup
WorkingDirectory=/var/lib/My App
"#
    );
    assert_eq!(
        eval(&js, "globalThis.timer"),
        r#"[Timer]
OnCalendar=Mon,Wed *-*-* 02:05:00
Persistent=true
Unit=nightly-backup.service

[Install]
WantedBy=timers.target
"#
    );
}

#[test]
fn systemd_preserves_unicode_and_rejects_ambiguous_values() {
    let js = runtime();
    run(
        &js,
        "unicode.js",
        r#"
        import { systemdService } from 'neo:os/schedulers/systemd';
        globalThis.service = systemdService({
          name: 'unicode-task',
          program: '/opt/日本語/アプリ',
          args: ['héllo', '🎉', 'a b'],
        });
        "#,
    );
    assert_eq!(
        eval(&js, "globalThis.service"),
        "[Service]\nType=oneshot\nExecStart=\"/opt/日本語/アプリ\" \"héllo\" \"🎉\" \"a b\"\n"
    );

    run(
        &js,
        "rejections.js",
        r#"
        import {
          SchedulerError,
          systemdService,
          systemdTimer,
          systemdUnits,
        } from 'neo:os/schedulers/systemd';
        function expect(code, field, fn) {
          let caught;
          try { fn(); } catch (error) { caught = error; }
          if (!caught) throw new Error('expected an error for ' + field);
          if (!(caught instanceof SchedulerError)) throw new Error('wrong error class for ' + field);
          if (caught.code !== code) throw new Error('wrong code for ' + field + ': ' + caught.code);
          if (caught.field !== field) throw new Error('wrong field: ' + caught.field);
        }
        const base = { name: 'task', program: '/bin/true' };
        expect('SCHEDULER_TYPE', 'options', () => systemdService(null));
        expect('SCHEDULER_VALUE', 'name', () => systemdService({ program: '/bin/true' }));
        expect('SCHEDULER_TYPE', 'name', () => systemdService({ name: 1, program: '/bin/true' }));
        expect('SCHEDULER_UNSUPPORTED', 'name', () => systemdService({ name: 'a/b', program: '/bin/true' }));
        expect('SCHEDULER_VALUE', 'name', () => systemdService({ name: ' a', program: '/bin/true' }));
        expect('SCHEDULER_UNSUPPORTED', 'name', () => systemdService({ name: 'task.service', program: '/bin/true' }));
        expect('SCHEDULER_VALUE', 'program', () => systemdService({ name: 'task', program: 'relative' }));
        expect('SCHEDULER_VALUE', 'program', () => systemdService({ name: 'task', program: '/bin/true\nExecStart=/bin/false' }));
        expect('SCHEDULER_UNSUPPORTED', 'program', () => systemdService({ name: 'task', program: '/bin/$x' }));
        expect('SCHEDULER_TYPE', 'args', () => systemdService({ ...base, args: 'x' }));
        expect('SCHEDULER_TYPE', 'args[0]', () => systemdService({ ...base, args: [1] }));
        expect('SCHEDULER_VALUE', 'args[0]', () => systemdService({ ...base, args: ['a\nb'] }));
        expect('SCHEDULER_UNSUPPORTED', 'args[0]', () => systemdService({ ...base, args: [';'] }));
        expect('SCHEDULER_VALUE', 'args[0]', () => systemdService({ ...base, args: ['\ud800'] }));
        expect('SCHEDULER_VALUE', 'args[0]', () => systemdService({ ...base, args: ['\ufffe'] }));
        expect('SCHEDULER_VALUE', 'description', () => systemdService({ ...base, description: 'a\nb' }));
        expect('SCHEDULER_UNSUPPORTED', 'description', () => systemdService({ ...base, description: '100%' }));
        expect('SCHEDULER_UNSUPPORTED', 'description', () => systemdService({ ...base, description: 'a\\b' }));
        expect('SCHEDULER_VALUE', 'workingDirectory', () => systemdService({ ...base, workingDirectory: 'relative' }));
        expect('SCHEDULER_UNSUPPORTED', 'workingDirectory', () => systemdService({ ...base, workingDirectory: '/tmp/100%' }));
        expect('SCHEDULER_VALUE', 'workingDirectory', () => systemdService({ ...base, workingDirectory: '/tmp/a\nb' }));
        expect('SCHEDULER_UNSUPPORTED', 'user', () => systemdService({ ...base, user: 'bad user' }));
        expect('SCHEDULER_TYPE', 'schedule', () => systemdTimer({ name: 'task' }));
        expect('SCHEDULER_SCHEDULE', 'schedule.kind', () => systemdTimer({ name: 'task', schedule: { kind: 'monthly', at: '09:30' } }));
        expect('SCHEDULER_SCHEDULE', 'schedule.at', () => systemdTimer({ name: 'task', schedule: { kind: 'daily', at: '24:00' } }));
        expect('SCHEDULER_SCHEDULE', 'schedule.at', () => systemdTimer({ name: 'task', schedule: { kind: 'daily', at: '9:30' } }));
        expect('SCHEDULER_SCHEDULE', 'schedule.at', () => systemdTimer({ name: 'task', schedule: { kind: 'daily', at: '09:60' } }));
        expect('SCHEDULER_TYPE', 'schedule.at', () => systemdTimer({ name: 'task', schedule: { kind: 'daily', at: 930 } }));
        expect('SCHEDULER_SCHEDULE', 'schedule.weekdays', () => systemdTimer({ name: 'task', schedule: { kind: 'daily', at: '09:30', weekdays: ['mon'] } }));
        expect('SCHEDULER_TYPE', 'schedule.weekdays', () => systemdTimer({ name: 'task', schedule: { kind: 'weekly', at: '09:30' } }));
        expect('SCHEDULER_VALUE', 'schedule.weekdays', () => systemdTimer({ name: 'task', schedule: { kind: 'weekly', at: '09:30', weekdays: [] } }));
        expect('SCHEDULER_VALUE', 'schedule.weekdays[0]', () => systemdTimer({ name: 'task', schedule: { kind: 'weekly', at: '09:30', weekdays: ['Mon'] } }));
        expect('SCHEDULER_VALUE', 'schedule.weekdays[1]', () => systemdTimer({ name: 'task', schedule: { kind: 'weekly', at: '09:30', weekdays: ['mon', 'mon'] } }));
        expect('SCHEDULER_TYPE', 'persistent', () => systemdTimer({ name: 'task', schedule: { kind: 'daily', at: '09:30' }, persistent: 'yes' }));
        expect('SCHEDULER_UNSUPPORTED', 'schedule', () => systemdService({ ...base, schedule: { kind: 'daily', at: '09:30' } }));
        expect('SCHEDULER_UNSUPPORTED', 'program', () => systemdTimer({ name: 'task', program: '/bin/true', schedule: { kind: 'daily', at: '09:30' } }));
        expect('SCHEDULER_UNSUPPORTED', 'interval', () => systemdUnits({ ...base, interval: 5, schedule: { kind: 'daily', at: '09:30' } }));
        expect('SCHEDULER_UNSUPPORTED', 'schedule.interval', () => systemdTimer({ name: 'task', schedule: { kind: 'daily', at: '09:30', interval: 5 } }));
        "#,
    );
}

#[test]
fn systemd_limits_are_reported_without_echoing_input() {
    let js = runtime();
    run(
        &js,
        "limits.js",
        r#"
        import { SchedulerError, systemdService } from 'neo:os/schedulers/systemd';
        function expectLimit(field, fn) {
          let caught;
          try { fn(); } catch (error) { caught = error; }
          if (!(caught instanceof SchedulerError)) throw new Error('wrong error for ' + field);
          if (caught.code !== 'SCHEDULER_LIMIT') throw new Error('wrong code for ' + field + ': ' + caught.code);
          if (caught.field !== field) throw new Error('wrong field: ' + caught.field);
          if (String(caught.message).includes('secret')) throw new Error('message echoed input');
        }
        expectLimit('name', () => systemdService({
          name: 'a'.repeat(201),
          program: '/bin/true',
        }));
        expectLimit('description', () => systemdService({
          name: 'task',
          program: '/bin/true',
          description: 'a'.repeat(32768),
        }));
        expectLimit('args[0]', () => systemdService({
          name: 'task',
          program: '/bin/true',
          args: ['a'.repeat(131073)],
        }));
        const args = [];
        for (let index = 0; index < 9; index += 1) args.push('a'.repeat(131072));
        expectLimit('ExecStart', () => systemdService({
          name: 'task',
          program: '/bin/true',
          args,
        }));
        "#,
    );
}

#[test]
fn windows_daily_task_is_exact() {
    let js = runtime();
    run(
        &js,
        "windows-daily.js",
        r#"
        import { windowsTask } from 'neo:os/schedulers/windows';
        globalThis.xml = windowsTask({
          name: 'Backup',
          description: 'Nightly backup',
          startDate: '2026-11-18',
          program: 'C:\\Program Files\\Neo\\backup.exe',
          args: ['--full', 'C:\\Data Folder'],
          workingDirectory: 'C:\\Program Files\\Neo',
          schedule: { kind: 'daily', at: '02:30' },
          principal: {
            userId: 'CONTOSO\\svc-backup',
            logonType: 'Password',
            runLevel: 'LeastPrivilege',
          },
        });
        "#,
    );
    assert_eq!(
        eval(&js, "globalThis.xml"),
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Nightly backup</Description>
    <URI>\Backup</URI>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <Enabled>true</Enabled>
      <StartBoundary>2026-11-18T02:30:00</StartBoundary>
      <ScheduleByDay>
        <DaysInterval>1</DaysInterval>
      </ScheduleByDay>
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>CONTOSO\svc-backup</UserId>
      <LogonType>Password</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Actions Context="Author">
    <Exec>
      <Command>C:\Program Files\Neo\backup.exe</Command>
      <Arguments>--full &quot;C:\Data Folder&quot;</Arguments>
      <WorkingDirectory>C:\Program Files\Neo</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    );
}

#[test]
fn windows_weekly_task_escapes_xml_and_quotes_arguments() {
    let js = runtime();
    run(
        &js,
        "windows-weekly.js",
        r#"
        import { windowsTask } from 'neo:os/schedulers/windows';
        globalThis.xml = windowsTask({
          name: 'A & B <C>',
          description: 'A & B < C > D "E" \'F\'',
          startDate: '2024-02-29',
          program: 'C:\\Tools\\a&b.exe',
          args: ['a b', '', 'quote"x', 'back\\', 'back\\"quote', 'a&b', 'x<y'],
          schedule: { kind: 'weekly', at: '23:59', weekdays: ['sun', 'tue'] },
        });
        "#,
    );
    assert_eq!(
        eval(&js, "globalThis.xml"),
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>A &amp; B &lt; C &gt; D &quot;E&quot; &apos;F&apos;</Description>
    <URI>\A &amp; B &lt;C&gt;</URI>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <Enabled>true</Enabled>
      <StartBoundary>2024-02-29T23:59:00</StartBoundary>
      <ScheduleByWeek>
        <DaysOfWeek>
          <Tuesday />
          <Sunday />
        </DaysOfWeek>
        <WeeksInterval>1</WeeksInterval>
      </ScheduleByWeek>
    </CalendarTrigger>
  </Triggers>
  <Actions>
    <Exec>
      <Command>C:\Tools\a&amp;b.exe</Command>
      <Arguments>&quot;a b&quot; &quot;&quot; &quot;quote\&quot;x&quot; back\ &quot;back\\\&quot;quote&quot; a&amp;b x&lt;y</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    );
}

#[test]
fn windows_rejects_ambiguous_paths_dates_and_principals() {
    let js = runtime();
    run(
        &js,
        "windows-rejections.js",
        r#"
        import { SchedulerError, windowsTask } from 'neo:os/schedulers/windows';
        function expect(code, field, fn) {
          let caught;
          try { fn(); } catch (error) { caught = error; }
          if (!caught) throw new Error('expected an error for ' + field);
          if (!(caught instanceof SchedulerError)) throw new Error('wrong error class for ' + field);
          if (caught.code !== code) throw new Error('wrong code for ' + field + ': ' + caught.code);
          if (caught.field !== field) throw new Error('wrong field: ' + caught.field);
        }
        const base = {
          name: 'Task',
          startDate: '2026-11-18',
          program: 'C:\\app.exe',
          schedule: { kind: 'daily', at: '09:30' },
        };
        expect('SCHEDULER_TYPE', 'options', () => windowsTask(null));
        expect('SCHEDULER_VALUE', 'name', () => windowsTask({ ...base, name: undefined }));
        expect('SCHEDULER_UNSUPPORTED', 'name', () => windowsTask({ ...base, name: 'folder\\task' }));
        expect('SCHEDULER_VALUE', 'name', () => windowsTask({ ...base, name: '..' }));
        expect('SCHEDULER_TYPE', 'startDate', () => windowsTask({ ...base, startDate: undefined }));
        expect('SCHEDULER_VALUE', 'startDate', () => windowsTask({ ...base, startDate: '2026-02-30' }));
        expect('SCHEDULER_VALUE', 'startDate', () => windowsTask({ ...base, startDate: '2026-13-01' }));
        expect('SCHEDULER_VALUE', 'startDate', () => windowsTask({ ...base, startDate: '2026-1-01' }));
        expect('SCHEDULER_VALUE', 'startDate', () => windowsTask({ ...base, startDate: '0000-01-01' }));
        expect('SCHEDULER_VALUE', 'program', () => windowsTask({ ...base, program: 'app.exe' }));
        expect('SCHEDULER_VALUE', 'program', () => windowsTask({ ...base, program: 'C:/app.exe' }));
        expect('SCHEDULER_VALUE', 'program', () => windowsTask({ ...base, program: 'C:app.exe' }));
        expect('SCHEDULER_UNSUPPORTED', 'program', () => windowsTask({ ...base, program: '\\\\?\\C:\\app.exe' }));
        expect('SCHEDULER_VALUE', 'program', () => windowsTask({ ...base, program: '\\\\server \\share\\app.exe' }));
        expect('SCHEDULER_UNSUPPORTED', 'program', () => windowsTask({ ...base, program: 'C:\\100%\\app.exe' }));
        expect('SCHEDULER_VALUE', 'program', () => windowsTask({ ...base, program: 'C:\\a\\..\\app.exe' }));
        expect('SCHEDULER_VALUE', 'program', () => windowsTask({ ...base, program: 'C:\\a\\\\app.exe' }));
        expect('SCHEDULER_VALUE', 'program', () => windowsTask({ ...base, program: 'C:\\app.exe\\' }));
        expect('SCHEDULER_VALUE', 'program', () => windowsTask({ ...base, program: 'C:\\app.exe\n' }));
        expect('SCHEDULER_LIMIT', 'program', () => windowsTask({ ...base, program: 'C:\\' + 'a'.repeat(258) }));
        expect('SCHEDULER_LIMIT', 'workingDirectory', () => windowsTask({ ...base, workingDirectory: 'C:\\' + 'a'.repeat(258) }));
        expect('SCHEDULER_VALUE', 'name', () => windowsTask({ ...base, name: 'a\nb' }));
        expect('SCHEDULER_VALUE', 'description', () => windowsTask({ ...base, description: 'a\nb' }));
        expect('SCHEDULER_VALUE', 'args[0]', () => windowsTask({ ...base, args: ['a\nb'] }));
        expect('SCHEDULER_TYPE', 'args', () => windowsTask({ ...base, args: 'x' }));
        expect('SCHEDULER_VALUE', 'principal.userId', () => windowsTask({ ...base, principal: { logonType: 'InteractiveToken' } }));
        expect('SCHEDULER_VALUE', 'principal.logonType', () => windowsTask({ ...base, principal: { userId: 'user', logonType: 'Unknown' } }));
        expect('SCHEDULER_VALUE', 'principal.runLevel', () => windowsTask({ ...base, principal: { userId: 'user', logonType: 'S4U', runLevel: 'Super' } }));
        expect('SCHEDULER_VALUE', 'principal.logonType', () => windowsTask({ ...base, principal: { userId: 'group', logonType: 'Group', runLevel: 'LeastPrivilege' } }));
        expect('SCHEDULER_VALUE', 'principal.logonType', () => windowsTask({ ...base, principal: { userId: 'SYSTEM', logonType: 'ServiceAccount' } }));
        expect('SCHEDULER_UNSUPPORTED', 'principal.groupId', () => windowsTask({ ...base, principal: { userId: 'user', logonType: 'S4U', groupId: 'group' } }));
        expect('SCHEDULER_UNSUPPORTED', 'enabled', () => windowsTask({ ...base, enabled: true }));
        expect('SCHEDULER_UNSUPPORTED', 'schedule.interval', () => windowsTask({ ...base, schedule: { kind: 'daily', at: '09:30', interval: 5 } }));
        expect('SCHEDULER_UNSUPPORTED', 'schedule.timezone', () => windowsTask({ ...base, schedule: { kind: 'weekly', at: '09:30', weekdays: ['mon'], timezone: 'UTC' } }));
        expect('SCHEDULER_LIMIT', 'commandLine', () => windowsTask({ ...base, args: ['a'.repeat(32767)] }));
        expect('SCHEDULER_LIMIT', 'args[0]', () => windowsTask({ ...base, args: ['a'.repeat(131073)] }));
        "#,
    );
}

#[test]
fn windows_accepts_unc_and_directory_roots_and_escapes_unicode() {
    let js = runtime();
    run(
        &js,
        "windows-accept.js",
        r#"
        import { windowsTask } from 'neo:os/schedulers/windows';
        globalThis.xml = windowsTask({
          name: '工具 🎉',
          description: '日本語 & <ok>',
          startDate: '2024-02-29',
          program: '\\\\server\\share\\工具\\アプリ.exe',
          args: ['héllo', '🎉'],
          workingDirectory: 'C:\\',
          schedule: { kind: 'weekly', at: '00:00', weekdays: ['mon', 'sun'] },
        });
        "#,
    );
    let xml = eval(&js, "globalThis.xml");
    assert!(xml.contains("<Command>\\\\server\\share\\工具\\アプリ.exe</Command>"));
    assert!(xml.contains("<Arguments>héllo 🎉</Arguments>"));
    assert!(xml.contains("<WorkingDirectory>C:\\</WorkingDirectory>"));
    assert!(xml.contains("<URI>\\工具 🎉</URI>"));
    assert!(xml.contains("<Description>日本語 &amp; &lt;ok&gt;</Description>"));
    assert!(xml.contains("<Monday />"));
    assert!(xml.contains("<Sunday />"));
}

fn utf16le_with_bom(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

#[test]
fn systemd_analyze_verifies_generated_units_if_available() {
    if !Path::new("/bin/echo").exists() {
        eprintln!("skipping: /bin/echo is unavailable");
        return;
    }
    if Command::new("systemd-analyze")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: systemd-analyze is unavailable");
        return;
    }

    let js = runtime();
    run(
        &js,
        "verify.js",
        r#"
        import { systemdUnits } from 'neo:os/schedulers/systemd';
        const units = systemdUnits({
          name: 'neo-scheduler-check',
          description: 'Verification probe',
          program: '/bin/echo',
          args: ['a b', '100%', '$HOME', 'quote"x', 'back\\slash', ''],
          workingDirectory: '/tmp/My App',
          user: 'root',
          schedule: { kind: 'weekly', at: '09:30', weekdays: ['mon', 'wed'] },
          persistent: true,
        });
        globalThis.service = units.service;
        globalThis.timer = units.timer;
        "#,
    );
    let service = eval(&js, "globalThis.service");
    let timer = eval(&js, "globalThis.timer");

    let directory = tempfile::tempdir().unwrap();
    let service_path = directory.path().join("neo-scheduler-check.service");
    let timer_path = directory.path().join("neo-scheduler-check.timer");
    std::fs::write(&service_path, &service).unwrap();
    std::fs::write(&timer_path, &timer).unwrap();

    let output = Command::new("systemd-analyze")
        .arg("verify")
        .arg(&service_path)
        .arg(&timer_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "systemd-analyze rejected generated units\nstdout: {}\nstderr: {}\nservice:\n{}\ntimer:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        service,
        timer
    );

    // An unescaped specifier is a real parse error, so the check above is not
    // vacuous. Our generator doubled the percent sign; replace the doubled
    // form with an unknown specifier and require systemd-analyze to reject it.
    let broken_specifier = service.replace("100%%", "100%z");
    assert_ne!(broken_specifier, service);
    let broken_path = directory.path().join("neo-scheduler-check.service");
    std::fs::write(&broken_path, &broken_specifier).unwrap();
    let broken = Command::new("systemd-analyze")
        .arg("verify")
        .arg(&broken_path)
        .arg(&timer_path)
        .output()
        .unwrap();
    assert!(
        !broken.status.success(),
        "systemd-analyze unexpectedly accepted an unescaped specifier"
    );

    // Unbalanced quoting is also rejected, confirming that verify parses the
    // ExecStart line rather than merely checking that the file exists.
    std::fs::write(
        &broken_path,
        "[Service]\nType=oneshot\nExecStart=\"/bin/echo\n",
    )
    .unwrap();
    let unbalanced = Command::new("systemd-analyze")
        .arg("verify")
        .arg(&broken_path)
        .output()
        .unwrap();
    assert!(
        !unbalanced.status.success(),
        "systemd-analyze unexpectedly accepted unbalanced quoting"
    );
}

#[cfg(unix)]
#[test]
fn systemd_analyze_accepts_a_program_path_with_spaces() {
    use std::os::unix::fs::PermissionsExt;

    if !Path::new("/bin/sh").exists()
        || Command::new("systemd-analyze")
            .arg("--version")
            .output()
            .is_err()
    {
        eprintln!("skipping: /bin/sh or systemd-analyze is unavailable");
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("my program");
    std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let program = serde_json::to_string(program.to_str().unwrap()).unwrap();

    let js = runtime();
    run(
        &js,
        "space-program.js",
        &format!(
            r#"
            import {{ systemdService }} from 'neo:os/schedulers/systemd';
            globalThis.service = systemdService({{
              name: 'neo-scheduler-space-check',
              program: {program},
              args: ['a b'],
            }});
            "#
        ),
    );
    let service = eval(&js, "globalThis.service");
    let path = directory.path().join("neo-scheduler-space-check.service");
    std::fs::write(&path, &service).unwrap();
    let output = Command::new("systemd-analyze")
        .arg("verify")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "systemd-analyze rejected a quoted program path with a space\nservice:\n{}\nstdout: {}\nstderr: {}",
        service,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn option_records_require_own_enumerable_data_properties() {
    let js = runtime();
    run(
        &js,
        "record-shape.js",
        r#"
        import { SchedulerError, systemdService, systemdTimer } from 'neo:os/schedulers/systemd';
        import { SchedulerError as WindowsSchedulerError, windowsTask } from 'neo:os/schedulers/windows';

        function expect(code, field, fn, ErrorClass = SchedulerError) {
          let caught;
          try { fn(); } catch (error) { caught = error; }
          if (!caught) throw new Error('expected an error for ' + field);
          if (!(caught instanceof ErrorClass)) throw new Error('wrong error class for ' + field);
          if (caught.code !== code) throw new Error('wrong code for ' + field + ': ' + caught.code);
          if (caught.field !== field) throw new Error('wrong field: ' + caught.field);
        }

        const normal = { name: 'task', program: '/bin/true' };
        const nullProto = Object.create(null);
        nullProto.name = 'task';
        nullProto.program = '/bin/true';
        if (systemdService(normal) !== systemdService(nullProto)) {
          throw new Error('null-prototype record differed');
        }

        const nullSchedule = Object.create(null);
        nullSchedule.kind = 'daily';
        nullSchedule.at = '09:30';
        if (systemdTimer({ name: 'task', schedule: nullSchedule }).indexOf('OnCalendar=*-*-* 09:30:00') === -1) {
          throw new Error('null-prototype schedule failed');
        }

        class Record {
          constructor() {
            this.name = 'task';
            this.program = '/bin/true';
          }
        }
        expect('SCHEDULER_TYPE', 'options', () => systemdService(new Record()));
        expect('SCHEDULER_TYPE', 'options', () => systemdService(Object.create({ program: '/bin/true' })));
        expect('SCHEDULER_TYPE', 'options', () => systemdService(new Date()));
        expect('SCHEDULER_TYPE', 'options', () => systemdService([]));

        Object.defineProperty(Object.prototype, 'program', {
          value: '/bin/false',
          configurable: true,
          writable: true,
        });
        try {
          expect('SCHEDULER_TYPE', 'program', () => systemdService({ name: 'task' }));
        } finally {
          delete Object.prototype.program;
        }

        Object.defineProperty(Object.prototype, 'interval', {
          value: 5,
          configurable: true,
          enumerable: true,
        });
        try {
          expect('SCHEDULER_UNSUPPORTED', 'interval', () => systemdService({
            name: 'task',
            program: '/bin/true',
          }));
        } finally {
          delete Object.prototype.interval;
        }

        const hidden = { name: 'task', program: '/bin/true' };
        Object.defineProperty(hidden, 'args', { value: ['x'], enumerable: false });
        expect('SCHEDULER_UNSUPPORTED', 'args', () => systemdService(hidden));

        const hiddenUnknown = { name: 'task', program: '/bin/true' };
        Object.defineProperty(hiddenUnknown, 'interval', { value: 5, enumerable: false });
        expect('SCHEDULER_UNSUPPORTED', 'interval', () => systemdService(hiddenUnknown));

        const hiddenSchedule = { kind: 'daily', at: '09:30' };
        Object.defineProperty(hiddenSchedule, 'interval', { value: 5, enumerable: false });
        expect('SCHEDULER_UNSUPPORTED', 'schedule.interval', () => systemdTimer({
          name: 'task',
          schedule: hiddenSchedule,
        }));

        const symbolOptions = { name: 'task', program: '/bin/true' };
        symbolOptions[Symbol('interval')] = 5;
        expect('SCHEDULER_UNSUPPORTED', 'options', () => systemdService(symbolOptions));

        const symbolSchedule = { kind: 'daily', at: '09:30' };
        symbolSchedule[Symbol('x')] = 1;
        expect('SCHEDULER_UNSUPPORTED', 'schedule', () => systemdTimer({
          name: 'task',
          schedule: symbolSchedule,
        }));

        const base = {
          name: 'Task',
          startDate: '2026-11-18',
          program: 'C:\\app.exe',
          schedule: { kind: 'daily', at: '09:30' },
        };
        Object.defineProperty(Object.prototype, 'userId', {
          value: 'user',
          configurable: true,
          writable: true,
        });
        try {
          expect('SCHEDULER_VALUE', 'principal.userId', () => windowsTask({
            ...base,
            principal: { logonType: 'S4U' },
          }), WindowsSchedulerError);
        } finally {
          delete Object.prototype.userId;
        }

        const symbolPrincipal = { userId: 'user', logonType: 'S4U' };
        symbolPrincipal[Symbol('x')] = 1;
        expect('SCHEDULER_UNSUPPORTED', 'principal', () => windowsTask({
          ...base,
          principal: symbolPrincipal,
        }), WindowsSchedulerError);

        const nullPrincipal = Object.create(null);
        nullPrincipal.userId = 'user';
        nullPrincipal.logonType = 'S4U';
        if (windowsTask({ ...base, principal: nullPrincipal }).indexOf('<LogonType>S4U</LogonType>') === -1) {
          throw new Error('null-prototype principal failed');
        }

        const hiddenPrincipal = { userId: 'user', logonType: 'S4U' };
        Object.defineProperty(hiddenPrincipal, 'runLevel', {
          value: 'LeastPrivilege',
          enumerable: false,
        });
        expect('SCHEDULER_UNSUPPORTED', 'principal.runLevel', () => windowsTask({
          ...base,
          principal: hiddenPrincipal,
        }), WindowsSchedulerError);
        expect('SCHEDULER_TYPE', 'principal', () => windowsTask({
          ...base,
          principal: Object.create({ userId: 'user', logonType: 'S4U' }),
        }), WindowsSchedulerError);
        expect('SCHEDULER_TYPE', 'schedule', () => systemdTimer({
          name: 'task',
          schedule: Object.create({ kind: 'daily', at: '09:30' }),
        }));

        const accessor = { name: 'task' };
        Object.defineProperty(accessor, 'program', {
          get() { throw new Error('accessor invoked'); },
          enumerable: true,
          configurable: true,
        });
        expect('SCHEDULER_UNSUPPORTED', 'program', () => systemdService(accessor));

        const accessorSchedule = { kind: 'daily' };
        Object.defineProperty(accessorSchedule, 'at', {
          get() { throw new Error('accessor invoked'); },
          enumerable: true,
          configurable: true,
        });
        expect('SCHEDULER_UNSUPPORTED', 'schedule.at', () => systemdTimer({
          name: 'task',
          schedule: accessorSchedule,
        }));

        const accessorPrincipal = { logonType: 'S4U' };
        Object.defineProperty(accessorPrincipal, 'userId', {
          get() { throw new Error('accessor invoked'); },
          enumerable: true,
          configurable: true,
        });
        expect('SCHEDULER_UNSUPPORTED', 'principal.userId', () => windowsTask({
          ...base,
          principal: accessorPrincipal,
        }), WindowsSchedulerError);

        const accessorArgs = ['--x'];
        Object.defineProperty(accessorArgs, '0', {
          get() { throw new Error('array accessor invoked'); },
          enumerable: true,
          configurable: true,
        });
        expect('SCHEDULER_UNSUPPORTED', 'args[0]', () => systemdService({
          name: 'task',
          program: '/bin/true',
          args: accessorArgs,
        }));

        const hiddenArgs = ['--x'];
        Object.defineProperty(hiddenArgs, '0', {
          value: '--x',
          enumerable: false,
          configurable: true,
        });
        expect('SCHEDULER_UNSUPPORTED', 'args[0]', () => systemdService({
          name: 'task',
          program: '/bin/true',
          args: hiddenArgs,
        }));

        const accessorWeekdays = ['mon'];
        Object.defineProperty(accessorWeekdays, '0', {
          get() { throw new Error('array accessor invoked'); },
          enumerable: true,
          configurable: true,
        });
        expect('SCHEDULER_UNSUPPORTED', 'schedule.weekdays[0]', () => systemdTimer({
          name: 'task',
          schedule: { kind: 'weekly', at: '09:30', weekdays: accessorWeekdays },
        }));

        const proxy = new Proxy({ name: 'task', program: '/bin/true' }, {
          get() { throw new Error('proxy get trap invoked'); },
        });
        if (systemdService(proxy).indexOf('ExecStart="/bin/true"') === -1) {
          throw new Error('transparent proxy record failed');
        }

        const hiddenProxy = new Proxy({ name: 'task', program: '/bin/true' }, {
          ownKeys() { return ['name', 'program', 'interval']; },
          getOwnPropertyDescriptor(target, key) {
            if (key === 'interval') {
              return { value: 5, enumerable: false, configurable: true };
            }
            return Object.getOwnPropertyDescriptor(target, key);
          },
        });
        expect('SCHEDULER_UNSUPPORTED', 'interval', () => systemdService(hiddenProxy));

        const protoProxy = new Proxy({ name: 'task', program: '/bin/true' }, {
          getPrototypeOf() { return {}; },
        });
        expect('SCHEDULER_TYPE', 'options', () => systemdService(protoProxy));
        "#,
    );
}

#[test]
fn xmllint_validates_every_supported_task_shape_against_the_published_xsd() {
    let xsd = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("task-scheduler-1.3.xsd");
    assert!(xsd.is_file(), "missing schema fixture: {}", xsd.display());
    if Command::new("xmllint").arg("--version").output().is_err() {
        eprintln!("skipping: xmllint is unavailable");
        return;
    }

    let js = runtime();
    run(
        &js,
        "schema-cases.js",
        r#"
        import { windowsTask } from 'neo:os/schedulers/windows';
        const schedules = [
          { kind: 'daily', at: '09:30' },
          { kind: 'weekly', at: '09:30', weekdays: ['mon', 'fri'] },
        ];
        const modes = ['InteractiveToken', 'Password', 'S4U', 'InteractiveTokenOrPassword'];
        const runLevels = [undefined, 'LeastPrivilege', 'HighestAvailable'];
        const xmls = [];
        for (const schedule of schedules) {
          xmls.push(windowsTask({
            name: 'Default',
            startDate: '2026-11-18',
            program: 'C:\\app.exe',
            schedule,
          }));
          for (const mode of modes) {
            for (const runLevel of runLevels) {
              const principal = { userId: 'CONTOSO\\user', logonType: mode };
              if (runLevel !== undefined) principal.runLevel = runLevel;
              xmls.push(windowsTask({
                name: 'Mode',
                startDate: '2026-11-18',
                program: 'C:\\app.exe',
                schedule,
                principal,
              }));
            }
          }
        }
        xmls.push(windowsTask({
          name: 'A & B <C>',
          description: 'A & B < C > D "E"',
          startDate: '2024-02-29',
          program: 'C:\\Program Files\\Neo\\app.exe',
          args: ['a b', '', 'quote"x', 'back\\', 'a&b', 'x<y'],
          workingDirectory: 'C:\\Program Files\\Neo',
          schedule: { kind: 'weekly', at: '23:59', weekdays: ['sun', 'tue'] },
          principal: { userId: 'CONTOSO\\user', logonType: 'S4U', runLevel: 'HighestAvailable' },
        }));
        globalThis.xmls = xmls;
        globalThis.xmlCount = String(xmls.length);
        "#,
    );
    let count: usize = eval(&js, "globalThis.xmlCount").parse().unwrap();
    assert_eq!(count, 2 + 2 * 4 * 3 + 1);

    let directory = tempfile::tempdir().unwrap();
    for index in 0..count {
        let xml = eval(&js, &format!("globalThis.xmls[{index}]"));
        let path = directory.path().join(format!("task-{index}.xml"));
        std::fs::write(&path, utf16le_with_bom(&xml)).unwrap();
        let output = Command::new("xmllint")
            .arg("--noout")
            .arg("--schema")
            .arg(&xsd)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "schema validation failed for case {index}\nxml:\n{xml}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Prove this is schema validation, not merely a well-formedness check.
    // The XSD fixes Task/@version to 1.3 and resolves Actions/@Context through
    // a keyref to a principal id, so each mutation must fail.
    let default_xml = eval(&js, "globalThis.xmls[0]");
    for (label, invalid) in [
        (
            "version",
            default_xml.replace("version=\"1.3\"", "version=\"1.2\""),
        ),
        (
            "context",
            default_xml.replace("<Actions>", "<Actions Context=\"Author\">"),
        ),
    ] {
        assert_ne!(invalid, default_xml, "mutation {label} did not apply");
        let path = directory.path().join(format!("invalid-{label}.xml"));
        std::fs::write(&path, utf16le_with_bom(&invalid)).unwrap();
        let output = Command::new("xmllint")
            .arg("--noout")
            .arg("--schema")
            .arg(&xsd)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "schema unexpectedly accepted the {label} mutation"
        );
    }
}

#[test]
fn xmllint_accepts_generated_utf16_task_and_preserves_text() {
    if Command::new("xmllint").arg("--version").output().is_err() {
        eprintln!("skipping: xmllint is unavailable");
        return;
    }

    let js = runtime();
    run(
        &js,
        "xml.js",
        r#"
        import { windowsTask } from 'neo:os/schedulers/windows';
        globalThis.xml = windowsTask({
          name: 'XML Check',
          description: 'A & B < C > D "E"',
          startDate: '2024-02-29',
          program: 'C:\\Program Files\\Neo\\check.exe',
          args: ['a b', '', 'quote"x', 'back\\', 'back\\"quote', 'a&b', 'x<y'],
          schedule: { kind: 'weekly', at: '23:59', weekdays: ['sun', 'tue'] },
        });
        "#,
    );
    let xml = eval(&js, "globalThis.xml");
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n"));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("task.xml");
    std::fs::write(&path, utf16le_with_bom(&xml)).unwrap();

    let well_formed = Command::new("xmllint")
        .arg("--noout")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        well_formed.status.success(),
        "xmllint rejected generated XML\nstderr: {}",
        String::from_utf8_lossy(&well_formed.stderr)
    );

    let version = Command::new("xmllint")
        .arg("--xpath")
        .arg("string(/*[local-name()=\"Task\"]/@version)")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&version.stdout).trim(), "1.3");

    let namespace = Command::new("xmllint")
        .arg("--xpath")
        .arg("string(namespace-uri(/*[local-name()=\"Task\"]))")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&namespace.stdout).trim(),
        "http://schemas.microsoft.com/windows/2004/02/mit/task"
    );

    let arguments = Command::new("xmllint")
        .arg("--xpath")
        .arg("string(//*[local-name()=\"Arguments\"])")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&arguments.stdout).trim_end(),
        "\"a b\" \"\" \"quote\\\"x\" back\\ \"back\\\\\\\"quote\" a&b x<y"
    );
}
