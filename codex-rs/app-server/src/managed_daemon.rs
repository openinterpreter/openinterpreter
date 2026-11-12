//! Unix process setup specific to a managed app-server daemon.

use std::io;

const NOFILE_LIMIT: libc::rlim_t = 4096;

fn target_soft_limit(soft: libc::rlim_t, hard: libc::rlim_t) -> libc::rlim_t {
    soft.max(hard.min(NOFILE_LIMIT))
}

pub(crate) fn raise_nofile_limit() -> io::Result<()> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit initializes the stack-owned resource-limit structure.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let target = target_soft_limit(limit.rlim_cur, limit.rlim_max);
    if target == limit.rlim_cur {
        return Ok(());
    }
    limit.rlim_cur = target;
    // SAFETY: the requested soft limit does not exceed the inherited hard limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::target_soft_limit;

    #[test]
    fn soft_limit_is_raised_only_within_inherited_hard_limit() {
        assert_eq!(target_soft_limit(1024, 8192), 4096);
        assert_eq!(target_soft_limit(1024, 2048), 2048);
        assert_eq!(target_soft_limit(8192, 8192), 8192);
    }
}
