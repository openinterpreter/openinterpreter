def in_jupyter_notebook():
    try:
        from IPython import get_ipython

        if "IPKernelApp" in get_ipython().config:
            return True
    except (ImportError, AttributeError):
        # IPython not available or not running in a notebook
        return False
