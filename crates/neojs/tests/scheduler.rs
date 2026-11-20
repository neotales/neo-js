use std::process::Command;

#[test]
fn cli_imports_scheduler_without_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("scheduler.js");
    std::fs::write(
        &main,
        r#"
        import { systemdUnits } from 'neo:os/schedulers/systemd';
        const { windowsTask } = await import('neo:os/schedulers/windows');
        const units = systemdUnits({
          name: 'backup',
          program: '/bin/true',
          schedule: { kind: 'daily', at: '09:30' },
        });
        console.log(units.service + units.timer);
        console.log(windowsTask({
          name: 'Backup',
          startDate: '2026-11-18',
          program: 'C:\\Program Files\\Neo\\backup.exe',
          schedule: { kind: 'daily', at: '09:30' },
        }));
        "#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .arg(&main)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cli failed without permissions\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("[Service]\nType=oneshot\nExecStart=\"/bin/true\"\n"));
    assert!(stdout.contains("OnCalendar=*-*-* 09:30:00"));
    assert!(stdout.contains(
        "<Task version=\"1.3\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">"
    ));
    assert!(stdout.contains("<Command>C:\\Program Files\\Neo\\backup.exe</Command>"));
}

#[test]
fn cli_reports_scheduler_error_codes() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("scheduler-error.js");
    std::fs::write(
        &main,
        r#"
        import { systemdService } from 'neo:os/schedulers/systemd';
        try {
          systemdService({ name: 'task', program: 'relative' });
        } catch (error) {
          console.log(error.code + ':' + error.field);
        }
        "#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .arg(&main)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"SCHEDULER_VALUE:program\n");
}
