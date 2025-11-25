import os
from unittest import TestCase, mock
import pytest

from interpreter.core.async_core import AsyncInterpreter, Server, _server_dependencies_installed


@pytest.mark.skipif(not _server_dependencies_installed, reason="Server dependencies not installed")
class TestServerConstruction(TestCase):
    """
    Tests to make sure that the underlying server is configured correctly when constructing
    the Server object.
    """

    def test_host_and_port_defaults(self):
        """
        Tests that a Server object takes on the default host and port when
        a) no host and port are passed in, and
        b) no HOST and PORT are set.
        """
        with mock.patch.dict(os.environ, {}):
            s = Server(AsyncInterpreter())
            self.assertEqual(s.host, Server.DEFAULT_HOST)
            self.assertEqual(s.port, Server.DEFAULT_PORT)

    def test_host_and_port_passed_in(self):
        """
        Tests that a Server object takes on the passed-in host and port when they are passed-in,
        ignoring the surrounding HOST and PORT env vars.
        """
        host = "the-really-real-host"
        port = 2222

        with mock.patch.dict(
            os.environ,
            {"INTERPRETER_HOST": "this-is-supes-fake", "INTERPRETER_PORT": "9876"},
        ):
            sboth = Server(AsyncInterpreter(), host, port)
            self.assertEqual(sboth.host, host)
            self.assertEqual(sboth.port, port)

    def test_host_and_port_from_env_1(self):
        """
        Tests that the Server object takes on the HOST and PORT env vars as host and port when
        nothing has been passed in.
        """
        fake_host = "fake_host"
        fake_port = 1234

        with mock.patch.dict(
            os.environ,
            {"INTERPRETER_HOST": fake_host, "INTERPRETER_PORT": str(fake_port)},
        ):
            s = Server(AsyncInterpreter())
            self.assertEqual(s.host, fake_host)
            self.assertEqual(s.port, fake_port)
