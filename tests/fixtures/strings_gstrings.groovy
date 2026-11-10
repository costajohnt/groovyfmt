def user = 'ada'
def greeting = "Hello, ${user.capitalize()}!"
def dollar = "price: $amount"
def block = '''
    first line
        indented line
last line
'''
def template = """
  Dear ${user},
    your total is ${String.format('%.2f', total)}
"""
def single = 'it\'s'
